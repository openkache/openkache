//! Scriptable and interactive command-line access to an OpenKache server.

use std::fmt::Display;
use std::io::{self, IsTerminal, Read, Write};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Once;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use clap::{Parser, Subcommand, ValueEnum};
use comfy_table::{Table, presets::UTF8_FULL};
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
#[cfg(feature = "quic-compio")]
use openkache_client_core::LocalProtectedClient as LocalClient;
#[cfg(feature = "quic-quinn")]
use openkache_client_core::ProtectedClient as Client;
use openkache_client_core::value::{Compression, Encryption};
use openkache_client_core::{
    Certificate, ClientIdentity, DataProtectionKey, DeleteOutcome, Endpoint, GetOutcome,
    PrivateKey, ServerTrust, SetOptions, SetOutcome, StructuredValue, contract,
    encode_structured_value,
};
use owo_colors::OwoColorize;
use reedline::{
    ColumnarMenu, DefaultCompleter, DefaultPrompt, DefaultPromptSegment, Emacs, KeyCode,
    KeyModifiers, MenuBuilder, Reedline, ReedlineEvent, ReedlineMenu, Signal,
    default_emacs_keybindings,
};
use serde_json::Value;

#[cfg(all(feature = "quic-compio", feature = "quic-quinn"))]
compile_error!("enable exactly one CLI QUIC backend feature");

#[cfg(not(any(feature = "quic-compio", feature = "quic-quinn")))]
compile_error!("enable one CLI QUIC backend feature");

/// Errors reported by the command-line client.
#[derive(Debug, thiserror::Error)]
pub enum CliError {
    /// A client operation or connection failed.
    #[error(transparent)]
    Client(#[from] openkache_client_core::Error),
    /// Reading a certificate, key file, or stdin failed.
    #[error("I/O failed: {0}")]
    Io(#[from] io::Error),
    /// The command-line configuration is invalid.
    #[error("invalid configuration: {0}")]
    Configuration(String),
    /// A command is not valid in the current context.
    #[error("invalid command: {0}")]
    Command(String),
    /// Tokio could not complete a blocking terminal task.
    #[error("terminal task failed: {0}")]
    TerminalTask(String),
}

/// Result type used by the command-line client.
pub type Result<T> = std::result::Result<T, CliError>;

/// Renders a client failure with a terminal-aware diagnostic layout.
///
/// Errors are written to stderr so command stdout remains safe for values and
/// machine-readable responses.
///
/// # Arguments
///
/// * `error` - The client failure to render.
///
/// # Returns
///
/// Nothing. The formatted diagnostic is written to stderr.
pub fn report_error(error: &CliError) {
    report_message(error, "run `openkache-cli --help` for usage");
}

/// Renders a process-level client failure with the same diagnostic layout as
/// command failures.
///
/// # Arguments
///
/// * `message` - Human-readable runtime initialization or configuration error.
/// * `help` - Actionable follow-up guidance shown below the error.
///
/// # Returns
///
/// Nothing. The formatted diagnostic is written to stderr.
pub fn report_message(message: impl Display, help: &str) {
    static DIAGNOSTIC_HANDLER: Once = Once::new();
    DIAGNOSTIC_HANDLER.call_once(|| {
        let _ = miette::set_hook(Box::new(|_| {
            Box::new(
                miette::MietteHandlerOpts::new()
                    .terminal_links(true)
                    .build(),
            )
        }));
    });

    let report = miette::miette!(
        severity = miette::Severity::Error,
        code = "openkache::cli",
        help = help,
        "{}",
        message
    );
    anstream::eprintln!("{report:?}");
}

/// Output encoding for values returned by `get`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum OutputFormat {
    /// Decode bytes lossily as UTF-8 and append a newline.
    #[default]
    Text,
    /// Write the exact stored bytes without a trailing newline.
    Raw,
    /// Encode stored bytes as padded Base64 and append a newline.
    Base64,
}

/// Connection and value-format profile selected by the CLI.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum ConnectionProfile {
    /// The fixed Rust Gate 0 profile used by local development clients.
    ///
    /// This profile uses the public Gate 0 Item-ID root, namespace `1`,
    /// `StructuredValue-CBOR-v1`, and the explicit development TLS trust
    /// policy. It is the default so the CLI can talk to the Rust client and
    /// the RESP-backed prototype without additional flags.
    #[default]
    Gate0,
    /// Configurable compatibility profile for certificates, value keys, and
    /// conditional or expiring writes.
    Configured,
}

/// Top-level command-line options and command selection.
#[derive(Debug, Parser)]
#[command(
    name = "openkache-cli",
    version,
    about = "Command-line client for the OpenKache cache server",
    arg_required_else_help = true
)]
pub struct Arguments {
    /// Client profile. Gate 0 matches the maintained Rust client and the
    /// local RESP-backed prototype; configured retains the full compatibility
    /// surface.
    #[arg(
        long,
        env = "OPENKACHE_PROFILE",
        value_enum,
        default_value_t = ConnectionProfile::Gate0,
        value_name = "PROFILE"
    )]
    pub profile: ConnectionProfile,

    /// OpenKache server address as `HOST:PORT`.
    #[arg(
        long,
        env = "OPENKACHE_ADDRESS",
        default_value = "127.0.0.1:4433",
        value_name = "HOST:PORT"
    )]
    pub address: String,

    /// TLS server name used when `address` is a socket address.
    #[arg(long, env = "OPENKACHE_SERVER_NAME", value_name = "NAME")]
    pub server_name: Option<String>,

    /// DER, PEM certificate, or PEM certificate-chain file trusted for TLS.
    #[arg(long, env = "OPENKACHE_CERTIFICATE", value_name = "PATH")]
    pub certificate: Option<PathBuf>,

    /// DER or PEM client certificate chain for mutual TLS.
    #[arg(
        long,
        env = "OPENKACHE_CLIENT_CERTIFICATE",
        requires = "client_key",
        value_name = "PATH"
    )]
    pub client_certificate: Option<PathBuf>,

    /// DER or PEM client private key for mutual TLS.
    #[arg(
        long,
        env = "OPENKACHE_CLIENT_KEY",
        requires = "client_certificate",
        value_name = "PATH"
    )]
    pub client_key: Option<PathBuf>,

    /// Optional Base64-encoded 32-byte data-protection key.
    #[arg(
        long,
        env = "OPENKACHE_DATA_PROTECTION_KEY",
        conflicts_with = "data_protection_key_file",
        hide_env_values = true,
        value_name = "BASE64"
    )]
    pub data_protection_key: Option<String>,

    /// Optional file containing a Base64-encoded 32-byte data-protection key.
    #[arg(
        long,
        env = "OPENKACHE_DATA_PROTECTION_KEY_FILE",
        conflicts_with = "data_protection_key",
        value_name = "PATH"
    )]
    pub data_protection_key_file: Option<PathBuf>,

    /// Operation to perform.
    #[command(subcommand)]
    pub command: Command,
}

/// One-shot operation supported by the CLI.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Verify the connection with a round trip.
    Ping,
    /// Retrieve a value by application key.
    Get {
        /// Application key to retrieve.
        key: String,
        /// Encoding used for the returned value.
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        output: OutputFormat,
    },
    /// Store a value by application key.
    Set {
        /// Application key to store.
        key: String,
        /// UTF-8 value. Use `--value-stdin` for exact bytes or multiline input.
        #[arg(value_name = "VALUE")]
        value: Option<String>,
        /// Read the complete value from stdin.
        #[arg(long, conflicts_with = "value")]
        value_stdin: bool,
        /// Store only when the key is absent.
        #[arg(long, conflicts_with = "if_present")]
        if_absent: bool,
        /// Store only when the key is present.
        #[arg(long, conflicts_with = "if_absent")]
        if_present: bool,
        /// Relative expiration in milliseconds.
        #[arg(long, value_name = "MILLISECONDS")]
        ttl_ms: Option<u64>,
    },
    /// Delete a value by application key.
    #[command(alias = "del")]
    Delete {
        /// Application key to delete.
        key: String,
    },
    /// Print validated server statistics.
    #[command(name = "experimental_stats", visible_alias = "experimental-stats")]
    ExperimentalStats,
    /// Wait until prior mutations satisfy the durability barrier.
    #[command(name = "experimental_sync", visible_alias = "experimental-sync")]
    ExperimentalSync,
    /// Start an interactive command shell on one connection.
    Shell,
}

/// Runs one parsed CLI invocation.
///
/// # Arguments
///
/// * `arguments` - Parsed options and one-shot command.
///
/// # Returns
///
/// `Ok(())` after the command completes successfully.
///
/// # Errors
///
/// Returns configuration, transport, protocol, value, or terminal I/O errors.
pub async fn run(arguments: Arguments) -> Result<()> {
    let profile = profile_from_arguments(&arguments)?;
    validate_command(profile, &arguments.command)?;
    let address = arguments.address.clone();
    let connecting = progress_bar(format!("connecting to {address}"));
    let client = match connect(&arguments, profile).await {
        Ok(client) => client,
        Err(error) => {
            connecting.abandon();
            return Err(error);
        }
    };
    connecting.finish_with_message(format!("connected to {address}"));

    match arguments.command {
        Command::Shell => run_shell(&client, &address).await,
        command => execute_command(&client, command).await,
    }
}

enum ConnectedClient {
    #[cfg(feature = "quic-compio")]
    Compio(LocalClient, ConnectionProfile),
    #[cfg(feature = "quic-quinn")]
    Quinn(Client, ConnectionProfile),
}

impl ConnectedClient {
    fn profile(&self) -> ConnectionProfile {
        match self {
            #[cfg(feature = "quic-compio")]
            Self::Compio(_, profile) => *profile,
            #[cfg(feature = "quic-quinn")]
            Self::Quinn(_, profile) => *profile,
        }
    }

    async fn ping(&self) -> openkache_client_core::Result<()> {
        match self {
            #[cfg(feature = "quic-compio")]
            Self::Compio(client, _) => client.ping().await.map(|_| ()),
            #[cfg(feature = "quic-quinn")]
            Self::Quinn(client, _) => client.ping().await.map(|_| ()),
        }
    }

    async fn get(
        &self,
        application_key: &str,
    ) -> openkache_client_core::Result<GetOutcome<ClientValue>> {
        match self {
            #[cfg(feature = "quic-compio")]
            Self::Compio(client, ConnectionProfile::Gate0) => {
                match client.get_structured(application_key).await? {
                    GetOutcome::Found(value) => {
                        Ok(GetOutcome::Found(ClientValue::Structured(value)))
                    }
                    GetOutcome::NotFound => Ok(GetOutcome::NotFound),
                }
            }
            #[cfg(feature = "quic-compio")]
            Self::Compio(client, ConnectionProfile::Configured) => {
                match client.get(application_key.as_bytes()).await? {
                    GetOutcome::Found(value) => Ok(GetOutcome::Found(ClientValue::Raw(value))),
                    GetOutcome::NotFound => Ok(GetOutcome::NotFound),
                }
            }
            #[cfg(feature = "quic-quinn")]
            Self::Quinn(client, ConnectionProfile::Gate0) => {
                match client.get_structured(application_key).await? {
                    GetOutcome::Found(value) => {
                        Ok(GetOutcome::Found(ClientValue::Structured(value)))
                    }
                    GetOutcome::NotFound => Ok(GetOutcome::NotFound),
                }
            }
            #[cfg(feature = "quic-quinn")]
            Self::Quinn(client, ConnectionProfile::Configured) => {
                match client.get(application_key.as_bytes()).await? {
                    GetOutcome::Found(value) => Ok(GetOutcome::Found(ClientValue::Raw(value))),
                    GetOutcome::NotFound => Ok(GetOutcome::NotFound),
                }
            }
        }
    }

    async fn set(
        &self,
        application_key: &str,
        value: InputValue,
        options: SetOptions,
    ) -> openkache_client_core::Result<SetOutcome> {
        match self {
            #[cfg(feature = "quic-compio")]
            Self::Compio(client, ConnectionProfile::Gate0) => {
                client
                    .set_structured(application_key, value.into_structured(), options)
                    .await
            }
            #[cfg(feature = "quic-compio")]
            Self::Compio(client, ConnectionProfile::Configured) => {
                client
                    .set(application_key.as_bytes(), value.into_bytes(), options)
                    .await
            }
            #[cfg(feature = "quic-quinn")]
            Self::Quinn(client, ConnectionProfile::Gate0) => {
                client
                    .set_structured(application_key, value.into_structured(), options)
                    .await
            }
            #[cfg(feature = "quic-quinn")]
            Self::Quinn(client, ConnectionProfile::Configured) => {
                client
                    .set(application_key.as_bytes(), value.into_bytes(), options)
                    .await
            }
        }
    }

    async fn delete(&self, application_key: &str) -> openkache_client_core::Result<DeleteOutcome> {
        match self {
            #[cfg(feature = "quic-compio")]
            Self::Compio(client, ConnectionProfile::Gate0) => client.delete(application_key).await,
            #[cfg(feature = "quic-compio")]
            Self::Compio(client, ConnectionProfile::Configured) => {
                client.delete(application_key.as_bytes()).await
            }
            #[cfg(feature = "quic-quinn")]
            Self::Quinn(client, ConnectionProfile::Gate0) => client.delete(application_key).await,
            #[cfg(feature = "quic-quinn")]
            Self::Quinn(client, ConnectionProfile::Configured) => {
                client.delete(application_key.as_bytes()).await
            }
        }
    }

    async fn experimental_stats(&self) -> openkache_client_core::Result<String> {
        match self {
            #[cfg(feature = "quic-compio")]
            Self::Compio(client, _) => client.experimental_stats().await,
            #[cfg(feature = "quic-quinn")]
            Self::Quinn(client, _) => client.experimental_stats().await,
        }
    }

    async fn experimental_sync(&self) -> openkache_client_core::Result<()> {
        match self {
            #[cfg(feature = "quic-compio")]
            Self::Compio(client, _) => client.experimental_sync().await,
            #[cfg(feature = "quic-quinn")]
            Self::Quinn(client, _) => client.experimental_sync().await,
        }
    }
}

async fn connect(arguments: &Arguments, profile: ConnectionProfile) -> Result<ConnectedClient> {
    let endpoint = endpoint_from_arguments(arguments)?;
    let data_protection_key = match profile {
        ConnectionProfile::Gate0 => {
            Some(DataProtectionKey::from_bytes(contract::GATE0_ITEM_ID_ROOT))
        }
        ConnectionProfile::Configured => data_protection_key_from_arguments(arguments)?,
    };
    let trust = match profile {
        ConnectionProfile::Gate0 => ServerTrust::Insecure,
        ConnectionProfile::Configured => trust_from_arguments(arguments)?,
    };
    let identity = match profile {
        ConnectionProfile::Gate0 => None,
        ConnectionProfile::Configured => client_identity_from_arguments(arguments)?,
    };

    #[cfg(feature = "quic-compio")]
    {
        let mut builder = match profile {
            ConnectionProfile::Gate0 => LocalClient::builder(
                endpoint,
                data_protection_key.expect("Gate 0 always supplies its fixed root"),
            )
            .server_trust(trust)
            .compression(Compression::Disabled)
            .encryption(Encryption::Unprotected)
            .namespace_id(contract::GATE0_NAMESPACE_ID),
            ConnectionProfile::Configured => match data_protection_key {
                Some(key) => LocalClient::builder(endpoint, key).server_trust(trust),
                None => LocalClient::builder_unprotected(endpoint).server_trust(trust),
            },
        };
        if let Some(identity) = identity {
            builder = builder.client_identity(identity);
        }
        builder
            .connect()
            .await
            .map(|client| ConnectedClient::Compio(client, profile))
            .map_err(CliError::from)
    }

    #[cfg(feature = "quic-quinn")]
    {
        let mut builder = match profile {
            ConnectionProfile::Gate0 => Client::builder(
                endpoint,
                data_protection_key.expect("Gate 0 always supplies its fixed root"),
            )
            .server_trust(trust)
            .compression(Compression::Disabled)
            .encryption(Encryption::Unprotected)
            .namespace_id(contract::GATE0_NAMESPACE_ID),
            ConnectionProfile::Configured => match data_protection_key {
                Some(key) => Client::builder(endpoint, key).server_trust(trust),
                None => Client::builder_unprotected(endpoint).server_trust(trust),
            },
        };
        if let Some(identity) = identity {
            builder = builder.client_identity(identity);
        }
        builder
            .connect()
            .await
            .map(|client| ConnectedClient::Quinn(client, profile))
            .map_err(CliError::from)
    }
}

fn profile_from_arguments(arguments: &Arguments) -> Result<ConnectionProfile> {
    let profile = arguments.profile;
    let has_configured_option = arguments.server_name.is_some()
        || arguments.certificate.is_some()
        || arguments.client_certificate.is_some()
        || arguments.client_key.is_some()
        || arguments.data_protection_key.is_some()
        || arguments.data_protection_key_file.is_some();
    if profile == ConnectionProfile::Gate0 && has_configured_option {
        return Err(CliError::Configuration(
            "Gate 0 uses the fixed Rust/prototype profile; use --profile configured for certificate, client-identity, or data-protection-key options".to_string(),
        ));
    }
    Ok(profile)
}

fn validate_command(profile: ConnectionProfile, command: &Command) -> Result<()> {
    if profile != ConnectionProfile::Gate0 {
        return Ok(());
    }
    if let Command::Set {
        if_absent,
        if_present,
        ttl_ms,
        ..
    } = command
        && (*if_absent || *if_present || ttl_ms.is_some())
    {
        return Err(CliError::Command(
            "Gate 0 supports only unconditional SET; use --profile configured for --if-absent, --if-present, or --ttl-ms"
                .to_string(),
        ));
    }
    Ok(())
}

fn endpoint_from_arguments(arguments: &Arguments) -> Result<Endpoint> {
    let address = arguments.address.trim();
    if address.is_empty() {
        return Err(CliError::Configuration(
            "address must not be empty".to_string(),
        ));
    }

    if let Ok(socket_address) = address.parse::<SocketAddr>() {
        let server_name = arguments
            .server_name
            .clone()
            .unwrap_or_else(|| socket_address.ip().to_string());
        return Endpoint::from_socket_addr(socket_address, server_name).map_err(CliError::from);
    }

    if arguments.server_name.is_some() {
        return Err(CliError::Configuration(
            "--server-name can only override a socket address; put the TLS hostname in --address"
                .to_string(),
        ));
    }

    Endpoint::from_str(address).map_err(CliError::from)
}

fn data_protection_key_from_arguments(arguments: &Arguments) -> Result<Option<DataProtectionKey>> {
    let encoded = match (
        arguments.data_protection_key.as_deref(),
        arguments.data_protection_key_file.as_deref(),
    ) {
        (Some(value), None) => value.to_string(),
        (None, Some(path)) => std::fs::read_to_string(path)?,
        (None, None) => return Ok(None),
        (Some(_), Some(_)) => {
            return Err(CliError::Configuration(
                "data-protection key may be supplied only once".to_string(),
            ));
        }
    };

    let encoded = encoded.trim();
    if encoded.is_empty() {
        return Err(CliError::Configuration(
            "data-protection key must not be empty".to_string(),
        ));
    }
    DataProtectionKey::from_base64(encoded)
        .map(Some)
        .map_err(CliError::from)
}

fn trust_from_arguments(arguments: &Arguments) -> Result<ServerTrust> {
    let Some(path) = arguments.certificate.as_deref() else {
        return Ok(ServerTrust::System);
    };
    let bytes = std::fs::read(path)?;
    let certificates = Certificate::from_der_or_pem_chain(&bytes).map_err(CliError::from)?;
    Ok(ServerTrust::Custom(certificates))
}

fn client_identity_from_arguments(arguments: &Arguments) -> Result<Option<ClientIdentity>> {
    let (Some(certificate_path), Some(key_path)) = (
        arguments.client_certificate.as_deref(),
        arguments.client_key.as_deref(),
    ) else {
        return Ok(None);
    };
    let certificate_bytes = std::fs::read(certificate_path)?;
    let certificates =
        Certificate::from_der_or_pem_chain(&certificate_bytes).map_err(CliError::from)?;
    let key_bytes = std::fs::read(key_path)?;
    let key = PrivateKey::from_der_or_pem(&key_bytes).map_err(CliError::from)?;
    ClientIdentity::new(certificates, key)
        .map(Some)
        .map_err(CliError::from)
}

async fn execute_command(client: &ConnectedClient, command: Command) -> Result<()> {
    validate_command(client.profile(), &command)?;
    match command {
        Command::Ping => {
            client.ping().await?;
            print_status("PONG");
        }
        Command::Get { key, output } => match client.get(&key).await? {
            GetOutcome::Found(value) => write_value(&value, output)?,
            GetOutcome::NotFound => print_status("NOT_FOUND"),
        },
        Command::Set {
            key,
            value,
            value_stdin,
            if_absent,
            if_present,
            ttl_ms,
        } => {
            let value = value_from_arguments(value, value_stdin)?;
            let options = set_options(if_absent, if_present, ttl_ms)?;
            let outcome = client.set(&key, value, options).await?;
            print_status(set_outcome_label(outcome));
        }
        Command::Delete { key } => {
            let outcome = client.delete(&key).await?;
            print_status(delete_outcome_label(outcome));
        }
        Command::ExperimentalStats => print_stats(&client.experimental_stats().await?)?,
        Command::ExperimentalSync => {
            let syncing = progress_bar("waiting for durable mutations".to_owned());
            if let Err(error) = client.experimental_sync().await {
                syncing.abandon();
                return Err(error.into());
            }
            syncing.finish_with_message("mutations are durable");
            print_status("OK");
        }
        Command::Shell => {
            return Err(CliError::Command(
                "shell must be the top-level command".to_string(),
            ));
        }
    }
    Ok(())
}

fn print_status(status: &'static str) {
    match status {
        "NOT_FOUND" | "NOT_STORED" => anstream::println!("{}", status.yellow().bold()),
        _ => anstream::println!("{}", status.green().bold()),
    }
}

fn print_stats(payload: &str) -> Result<()> {
    if !io::stdout().is_terminal() {
        // Keep the existing JSON contract for pipes, scripts, and CI.
        anstream::println!("{payload}");
        return Ok(());
    }

    let value: Value = serde_json::from_str(payload).map_err(|error| {
        CliError::Command(format!(
            "EXPERIMENTAL_STATS response is not valid JSON: {error}"
        ))
    })?;
    let mut table = Table::new();
    table.load_preset(UTF8_FULL);
    table.set_header(["metric", "value"]);
    add_stat_rows(&mut table, "", &value);

    anstream::println!("{}", "OpenKache experimental_stats".bold());
    anstream::println!("{table}");
    Ok(())
}

fn add_stat_rows(table: &mut Table, prefix: &str, value: &Value) {
    match value {
        Value::Object(fields) => {
            for (name, value) in fields {
                let path = if prefix.is_empty() {
                    name.to_owned()
                } else {
                    format!("{prefix}.{name}")
                };
                add_stat_rows(table, &path, value);
            }
        }
        Value::Array(values) => {
            if values.is_empty() {
                table.add_row(vec![prefix.to_owned(), "[]".to_owned()]);
            } else {
                for (index, value) in values.iter().enumerate() {
                    let path = format!("{prefix}[{index}]");
                    add_stat_rows(table, &path, value);
                }
            }
        }
        _ => {
            table.add_row(vec![prefix.to_owned(), stat_value(value)]);
        }
    }
}

fn stat_value(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        _ => value.to_string(),
    }
}

fn progress_bar(message: String) -> ProgressBar {
    let progress = ProgressBar::new_spinner();
    if !io::stderr().is_terminal() {
        progress.set_draw_target(ProgressDrawTarget::hidden());
        return progress;
    }

    let style = ProgressStyle::with_template("{spinner:.green} {msg}")
        .unwrap_or_else(|_| ProgressStyle::default_spinner())
        .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]);
    progress.set_style(style);
    progress.enable_steady_tick(Duration::from_millis(90));
    progress.set_message(message);
    progress
}

enum ClientValue {
    Raw(Vec<u8>),
    Structured(StructuredValue),
}

impl ClientValue {
    fn output_bytes(&self) -> Result<Vec<u8>> {
        match self {
            Self::Raw(value) => Ok(value.clone()),
            Self::Structured(StructuredValue::TextString(value)) => Ok(value.as_bytes().to_vec()),
            Self::Structured(StructuredValue::Bytes(value)) => Ok(value.clone()),
            Self::Structured(value) => encode_structured_value(value).map_err(|error| {
                CliError::Command(format!(
                    "could not encode the structured value for CLI output: {error}"
                ))
            }),
        }
    }
}

enum InputValue {
    Text(String),
    Bytes(Vec<u8>),
}

impl InputValue {
    fn into_bytes(self) -> Vec<u8> {
        match self {
            Self::Text(value) => value.into_bytes(),
            Self::Bytes(value) => value,
        }
    }

    fn into_structured(self) -> StructuredValue {
        match self {
            Self::Text(value) => StructuredValue::text(value),
            Self::Bytes(value) => StructuredValue::bytes(value),
        }
    }
}

fn value_from_arguments(value: Option<String>, value_stdin: bool) -> Result<InputValue> {
    match (value, value_stdin) {
        (Some(value), false) => Ok(InputValue::Text(value)),
        (None, true) => {
            let mut value = Vec::new();
            io::stdin().read_to_end(&mut value)?;
            Ok(InputValue::Bytes(value))
        }
        (Some(_), true) => Err(CliError::Command(
            "set accepts either VALUE or --value-stdin, not both".to_string(),
        )),
        (None, false) => Err(CliError::Command(
            "set requires VALUE or --value-stdin".to_string(),
        )),
    }
}

fn set_options(if_absent: bool, if_present: bool, ttl_ms: Option<u64>) -> Result<SetOptions> {
    if if_absent && if_present {
        return Err(CliError::Command(
            "--if-absent and --if-present are mutually exclusive".to_string(),
        ));
    }
    if ttl_ms == Some(0) {
        return Err(CliError::Command(
            "--ttl-ms must be greater than zero".to_string(),
        ));
    }

    let mut options = SetOptions::new();
    if if_absent {
        options = options.if_absent();
    } else if if_present {
        options = options.if_present();
    }
    if let Some(ttl_ms) = ttl_ms {
        options = options.expires_after_millis(ttl_ms);
    }
    Ok(options)
}

fn write_value(value: &ClientValue, output: OutputFormat) -> Result<()> {
    let value = value.output_bytes()?;
    let mut stdout = io::stdout().lock();
    match output {
        OutputFormat::Text => {
            stdout.write_all(String::from_utf8_lossy(&value).as_bytes())?;
            stdout.write_all(b"\n")?;
        }
        OutputFormat::Raw => stdout.write_all(&value)?,
        OutputFormat::Base64 => {
            stdout.write_all(STANDARD.encode(&value).as_bytes())?;
            stdout.write_all(b"\n")?;
        }
    }
    stdout.flush()?;
    Ok(())
}

fn set_outcome_label(outcome: SetOutcome) -> &'static str {
    match outcome {
        SetOutcome::Created => "CREATED",
        SetOutcome::Replaced => "REPLACED",
        SetOutcome::NotStored => "NOT_STORED",
    }
}

fn delete_outcome_label(outcome: DeleteOutcome) -> &'static str {
    match outcome {
        DeleteOutcome::Deleted => "DELETED",
        DeleteOutcome::NotFound => "NOT_FOUND",
    }
}

async fn run_shell(client: &ConnectedClient, address: &str) -> Result<()> {
    anstream::println!("{}", format!("Connected to {address}").green().bold());
    anstream::println!("{}", "Type 'help' for commands or 'exit' to quit.".dimmed());

    let mut line_editor = shell_editor();
    let prompt = DefaultPrompt::new(
        DefaultPromptSegment::Basic("openkache ❯ ".to_owned()),
        DefaultPromptSegment::Empty,
    );
    loop {
        let (editor, signal) = read_shell_signal_async(line_editor, prompt.clone()).await?;
        line_editor = editor;

        let line = match signal {
            Signal::Success(line) => line,
            Signal::CtrlD => {
                anstream::println!();
                break;
            }
            Signal::CtrlC => {
                anstream::eprintln!("{}", "input cancelled".yellow());
                continue;
            }
            _ => continue,
        };

        let command = match parse_shell_command(&line) {
            Ok(Some(command)) => command,
            Ok(None) => continue,
            Err(message) => {
                print_shell_error(&message);
                continue;
            }
        };
        match command {
            ShellCommand::Exit => break,
            ShellCommand::Help => print_shell_help(),
            ShellCommand::Command(command) => {
                if let Err(error) = execute_command(client, command).await {
                    print_shell_error(&error.to_string());
                }
            }
        }
    }
    Ok(())
}

fn shell_editor() -> Reedline {
    let commands = [
        "ping",
        "get",
        "set",
        "delete",
        "del",
        "experimental_stats",
        "experimental_sync",
        "help",
        "exit",
        "quit",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    let completer = Box::new(DefaultCompleter::new_with_wordlen(commands, 1));
    let completion_menu = Box::new(ColumnarMenu::default().with_name("completion_menu"));
    let mut keybindings = default_emacs_keybindings();
    keybindings.add_binding(
        KeyModifiers::NONE,
        KeyCode::Tab,
        ReedlineEvent::UntilFound(vec![
            ReedlineEvent::Menu("completion_menu".to_owned()),
            ReedlineEvent::MenuNext,
        ]),
    );
    let edit_mode = Box::new(Emacs::new(keybindings));

    Reedline::create()
        .with_completer(completer)
        .with_menu(ReedlineMenu::EngineCompleter(completion_menu))
        .with_edit_mode(edit_mode)
        .with_ansi_colors(std::env::var_os("NO_COLOR").is_none())
}

#[cfg(feature = "quic-compio")]
async fn read_shell_signal_async(
    editor: Reedline,
    prompt: DefaultPrompt,
) -> Result<(Reedline, Signal)> {
    let (editor, signal) = compio::runtime::spawn_blocking(move || {
        let mut editor = editor;
        let signal = editor.read_line(&prompt);
        (editor, signal)
    })
    .await
    .map_err(|error| CliError::TerminalTask(error.to_string()))?;
    let signal = signal.map_err(|error| CliError::TerminalTask(error.to_string()))?;
    Ok((editor, signal))
}

#[cfg(feature = "quic-quinn")]
async fn read_shell_signal_async(
    editor: Reedline,
    prompt: DefaultPrompt,
) -> Result<(Reedline, Signal)> {
    let (editor, signal) = tokio::task::spawn_blocking(move || {
        let mut editor = editor;
        let signal = editor.read_line(&prompt);
        (editor, signal)
    })
    .await
    .map_err(|error| CliError::TerminalTask(error.to_string()))?;
    let signal = signal.map_err(|error| CliError::TerminalTask(error.to_string()))?;
    Ok((editor, signal))
}

fn print_shell_error(message: &str) {
    anstream::eprintln!("{} {message}", "error:".red().bold());
}

fn print_shell_help() {
    let mut table = Table::new();
    table.load_preset(UTF8_FULL);
    table.set_header(["command", "description"]);
    table.add_row(["ping", "verify the connection"]);
    table.add_row(["get <key>", "retrieve a value"]);
    table.add_row(["set <key> <value>", "store a value"]);
    table.add_row(["delete <key>", "remove a value"]);
    table.add_row(["experimental_stats", "show server statistics"]);
    table.add_row(["experimental_sync", "wait for durable mutations"]);
    table.add_row(["exit / quit", "close the shell"]);
    anstream::println!("{table}");
}

enum ShellCommand {
    Command(Command),
    Help,
    Exit,
}

fn parse_shell_command(line: &str) -> std::result::Result<Option<ShellCommand>, String> {
    let line = line.trim();
    if line.is_empty() {
        return Ok(None);
    }
    let command_end = line.find(char::is_whitespace).unwrap_or(line.len());
    let command = line[..command_end].to_ascii_lowercase();
    let arguments = line[command_end..].trim();

    match command.as_str() {
        "ping" => {
            require_no_arguments(arguments, "ping")?;
            Ok(Some(ShellCommand::Command(Command::Ping)))
        }
        "get" => Ok(Some(ShellCommand::Command(Command::Get {
            key: parse_single_argument(arguments, "get <key>")?,
            output: OutputFormat::Text,
        }))),
        "set" => {
            let (key, value) = parse_set_arguments(arguments)?;
            Ok(Some(ShellCommand::Command(Command::Set {
                key,
                value: Some(value),
                value_stdin: false,
                if_absent: false,
                if_present: false,
                ttl_ms: None,
            })))
        }
        "delete" | "del" => Ok(Some(ShellCommand::Command(Command::Delete {
            key: parse_single_argument(arguments, "delete <key>")?,
        }))),
        "experimental_stats" => {
            require_no_arguments(arguments, "experimental_stats")?;
            Ok(Some(ShellCommand::Command(Command::ExperimentalStats)))
        }
        "experimental_sync" => {
            require_no_arguments(arguments, "experimental_sync")?;
            Ok(Some(ShellCommand::Command(Command::ExperimentalSync)))
        }
        "help" | "?" => {
            require_no_arguments(arguments, "help")?;
            Ok(Some(ShellCommand::Help))
        }
        "exit" | "quit" => {
            require_no_arguments(arguments, "exit")?;
            Ok(Some(ShellCommand::Exit))
        }
        _ => Err(format!("unknown command '{command}'; type 'help'")),
    }
}

fn require_no_arguments(arguments: &str, usage: &str) -> std::result::Result<(), String> {
    if arguments.is_empty() {
        Ok(())
    } else {
        Err(format!("usage: {usage}"))
    }
}

fn parse_single_argument(arguments: &str, usage: &str) -> std::result::Result<String, String> {
    let mut parts = arguments.split_whitespace();
    match (parts.next(), parts.next()) {
        (Some(argument), None) => Ok(argument.to_string()),
        _ => Err(format!("usage: {usage}")),
    }
}

fn parse_set_arguments(arguments: &str) -> std::result::Result<(String, String), String> {
    let Some(separator) = arguments.find(char::is_whitespace) else {
        return Err("usage: set <key> <value>".to_string());
    };
    let key = &arguments[..separator];
    let value = arguments[separator..].trim();
    if key.is_empty() || value.is_empty() {
        return Err("usage: set <key> <value>".to_string());
    }
    Ok((key.to_string(), value.to_string()))
}
