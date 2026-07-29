//! Small internal PKI for self-managed OpenKache deployments.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::net::IpAddr;
use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa,
    Issuer, KeyPair, KeyUsagePurpose,
};
use time::{Duration, OffsetDateTime};

const DEFAULT_DIRECTORY: &str = "_local/openkache-pki";
const DEFAULT_CA_VALID_DAYS: u32 = 3_650;
const DEFAULT_LEAF_VALID_DAYS: u32 = 365;

/// Optional top-level maintenance commands.
#[derive(Subcommand)]
pub(super) enum Command {
    /// Manage the OpenKache internal certificate authority.
    Pki(PkiArguments),
}

impl Command {
    pub(super) fn run(&self) -> Result<(), PkiError> {
        match self {
            Self::Pki(arguments) => arguments.run(),
        }
    }
}

/// Internal certificate-authority commands.
#[derive(Args)]
pub(super) struct PkiArguments {
    /// PKI workspace containing the offline authority and deployable bundles.
    #[arg(long, global = true, default_value = DEFAULT_DIRECTORY, value_name = "PATH")]
    workspace: PathBuf,

    #[command(subcommand)]
    command: PkiCommand,
}

#[derive(Subcommand)]
enum PkiCommand {
    /// Create the OpenKache internal certificate authority.
    Init {
        /// CA certificate lifetime in days.
        #[arg(long, default_value_t = DEFAULT_CA_VALID_DAYS)]
        valid_days: u32,
    },

    /// Issue the server identity and deployable server bundle.
    IssueServer {
        /// DNS name included in the server certificate; repeatable.
        #[arg(long = "dns", value_name = "NAME")]
        dns_names: Vec<String>,

        /// IP address included in the server certificate; repeatable.
        #[arg(long = "ip", value_name = "ADDRESS")]
        ip_addresses: Vec<IpAddr>,

        /// Server certificate lifetime in days.
        #[arg(long, default_value_t = DEFAULT_LEAF_VALID_DAYS)]
        valid_days: u32,
    },

    /// Issue a regular application client identity.
    IssueClient {
        /// Stable client name used in the certificate and output directory.
        name: String,

        /// Client certificate lifetime in days.
        #[arg(long, default_value_t = DEFAULT_LEAF_VALID_DAYS)]
        valid_days: u32,
    },

    /// Issue an administrator identity and register its public certificate.
    IssueAdmin {
        /// Stable administrator name used in the certificate and output directory.
        name: String,

        /// Administrator certificate lifetime in days.
        #[arg(long, default_value_t = DEFAULT_LEAF_VALID_DAYS)]
        valid_days: u32,
    },

    /// List issued identities and server-bundle readiness.
    List,
}

impl PkiArguments {
    pub(super) fn run(&self) -> Result<(), PkiError> {
        match &self.command {
            PkiCommand::Init { valid_days } => initialize(&self.workspace, *valid_days),
            PkiCommand::IssueServer {
                dns_names,
                ip_addresses,
                valid_days,
            } => issue_server(&self.workspace, dns_names, ip_addresses, *valid_days),
            PkiCommand::IssueClient { name, valid_days } => {
                issue_client(&self.workspace, name, *valid_days)
            }
            PkiCommand::IssueAdmin { name, valid_days } => {
                issue_admin(&self.workspace, name, *valid_days)
            }
            PkiCommand::List => list(&self.workspace),
        }
    }
}

pub(super) fn initialize(directory: &Path, valid_days: u32) -> Result<(), PkiError> {
    let authority = directory.join("authority");
    let ca_certificate_path = authority.join("ca.crt");
    let ca_key_path = authority.join("ca.key");
    let server_ca_path = directory.join("server/ca.crt");
    ensure_absent([
        ca_certificate_path.as_path(),
        ca_key_path.as_path(),
        server_ca_path.as_path(),
    ])?;

    let (not_before, not_after) = validity(valid_days)?;
    let mut distinguished_name = DistinguishedName::new();
    distinguished_name.push(DnType::OrganizationName, "OpenKache");
    distinguished_name.push(DnType::CommonName, "OpenKache Internal CA");
    let mut params = CertificateParams::default();
    params.not_before = not_before;
    params.not_after = not_after;
    params.distinguished_name = distinguished_name;
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
    ];
    let key = KeyPair::generate()?;
    let certificate = params.self_signed(&key)?;
    let certificate_pem = certificate.pem();

    create_private_directory(directory)?;
    create_private_directory(&authority)?;
    create_private_directory(&directory.join("server"))?;
    create_private_directory(&directory.join("server/admins"))?;
    create_private_directory(&directory.join("clients"))?;
    create_private_directory(&directory.join("admins"))?;
    write_new(&ca_key_path, key.serialize_pem().as_bytes(), 0o600)?;
    write_new(&ca_certificate_path, certificate_pem.as_bytes(), 0o644)?;
    write_new(&server_ca_path, certificate_pem.as_bytes(), 0o644)?;

    println!("Created OpenKache internal CA in {}", authority.display());
    println!("Keep {} offline and private.", ca_key_path.display());
    println!("Next: issue-server, issue-client, and issue-admin.");
    Ok(())
}

pub(super) fn issue_server(
    directory: &Path,
    dns_names: &[String],
    ip_addresses: &[IpAddr],
    valid_days: u32,
) -> Result<(), PkiError> {
    let certificate_path = directory.join("server/server.crt");
    let key_path = directory.join("server/server.key");
    ensure_absent([certificate_path.as_path(), key_path.as_path()])?;

    let mut subject_alt_names = dns_names.to_vec();
    subject_alt_names.extend(ip_addresses.iter().map(ToString::to_string));
    if subject_alt_names.is_empty() {
        subject_alt_names = vec!["localhost".into(), "127.0.0.1".into(), "::1".into()];
    }
    let common_name = dns_names
        .first()
        .cloned()
        .or_else(|| ip_addresses.first().map(ToString::to_string))
        .unwrap_or_else(|| "localhost".into());
    let mut params = leaf_params(&common_name, valid_days)?;
    params.subject_alt_names = CertificateParams::new(subject_alt_names)?.subject_alt_names;
    params
        .extended_key_usages
        .push(ExtendedKeyUsagePurpose::ServerAuth);
    let (certificate, key) = sign(directory, params)?;

    write_new(&key_path, key.serialize_pem().as_bytes(), 0o600)?;
    write_new(&certificate_path, certificate.pem().as_bytes(), 0o644)?;
    println!("Issued server identity:");
    println!("  certificate: {}", certificate_path.display());
    println!("  private key: {}", key_path.display());
    Ok(())
}

pub(super) fn issue_client(directory: &Path, name: &str, valid_days: u32) -> Result<(), PkiError> {
    issue_client_identity(directory, name, valid_days, ClientRole::Application)
}

pub(super) fn issue_admin(directory: &Path, name: &str, valid_days: u32) -> Result<(), PkiError> {
    issue_client_identity(directory, name, valid_days, ClientRole::Administrator)
}

fn issue_client_identity(
    directory: &Path,
    name: &str,
    valid_days: u32,
    role: ClientRole,
) -> Result<(), PkiError> {
    validate_name(name)?;
    let bundle = directory.join(role.bundle_directory()).join(name);
    let certificate_path = bundle.join(role.certificate_name());
    let key_path = bundle.join(role.key_name());
    let ca_path = bundle.join("ca.crt");
    let registered_admin_path = role
        .is_administrator()
        .then(|| directory.join("server/admins").join(format!("{name}.crt")));
    let mut outputs = vec![
        certificate_path.as_path(),
        key_path.as_path(),
        ca_path.as_path(),
    ];
    if let Some(path) = &registered_admin_path {
        outputs.push(path.as_path());
    }
    ensure_absent(outputs)?;

    let mut params = leaf_params(name, valid_days)?;
    params
        .extended_key_usages
        .push(ExtendedKeyUsagePurpose::ClientAuth);
    let (certificate, key) = sign(directory, params)?;
    let certificate_pem = certificate.pem();
    let ca_pem = read_authority_certificate(directory)?;

    create_private_directory(&bundle)?;
    write_new(&key_path, key.serialize_pem().as_bytes(), 0o600)?;
    write_new(&certificate_path, certificate_pem.as_bytes(), 0o644)?;
    write_new(&ca_path, ca_pem.as_bytes(), 0o644)?;
    if let Some(path) = registered_admin_path {
        create_private_directory(
            path.parent()
                .expect("administrator registration path has a parent"),
        )?;
        write_new(&path, certificate_pem.as_bytes(), 0o644)?;
    }

    println!("Issued {} identity {name}:", role.label());
    println!("  bundle: {}", bundle.display());
    if role.is_administrator() {
        println!("  registered for STATS and SYNC");
    }
    Ok(())
}

fn leaf_params(common_name: &str, valid_days: u32) -> Result<CertificateParams, PkiError> {
    let (not_before, not_after) = validity(valid_days)?;
    let mut distinguished_name = DistinguishedName::new();
    distinguished_name.push(DnType::OrganizationName, "OpenKache");
    distinguished_name.push(DnType::CommonName, common_name);
    let mut params = CertificateParams::default();
    params.not_before = not_before;
    params.not_after = not_after;
    params.distinguished_name = distinguished_name;
    params.key_usages.push(KeyUsagePurpose::DigitalSignature);
    params.use_authority_key_identifier_extension = true;
    Ok(params)
}

fn sign(
    directory: &Path,
    params: CertificateParams,
) -> Result<(rcgen::Certificate, KeyPair), PkiError> {
    let ca_certificate = read_authority_certificate(directory)?;
    let ca_key_path = directory.join("authority/ca.key");
    let ca_key_pem = read_to_string(&ca_key_path)?;
    let ca_key = KeyPair::from_pem(&ca_key_pem)?;
    let issuer = Issuer::from_ca_cert_pem(&ca_certificate, ca_key)?;
    let key = KeyPair::generate()?;
    let certificate = params.signed_by(&key, &issuer)?;
    Ok((certificate, key))
}

fn read_authority_certificate(directory: &Path) -> Result<String, PkiError> {
    let path = directory.join("authority/ca.crt");
    if !path.is_file() {
        return Err(PkiError::NotInitialized(directory.to_path_buf()));
    }
    read_to_string(&path)
}

fn read_to_string(path: &Path) -> Result<String, PkiError> {
    fs::read_to_string(path).map_err(|source| PkiError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn validity(valid_days: u32) -> Result<(OffsetDateTime, OffsetDateTime), PkiError> {
    if valid_days == 0 {
        return Err(PkiError::InvalidValidity);
    }
    let now = OffsetDateTime::now_utc();
    let not_before = now
        .checked_sub(Duration::minutes(5))
        .ok_or(PkiError::InvalidValidity)?;
    let not_after = now
        .checked_add(Duration::days(i64::from(valid_days)))
        .ok_or(PkiError::InvalidValidity)?;
    Ok((not_before, not_after))
}

fn validate_name(name: &str) -> Result<(), PkiError> {
    let valid = !name.is_empty()
        && name.len() <= 64
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-_.".contains(character));
    if valid {
        Ok(())
    } else {
        Err(PkiError::InvalidName(name.into()))
    }
}

fn ensure_absent<'a>(paths: impl IntoIterator<Item = &'a Path>) -> Result<(), PkiError> {
    for path in paths {
        if path.exists() {
            return Err(PkiError::AlreadyExists(path.to_path_buf()));
        }
    }
    Ok(())
}

fn create_private_directory(path: &Path) -> Result<(), PkiError> {
    fs::create_dir_all(path).map_err(|source| PkiError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|source| {
            PkiError::Io {
                path: path.to_path_buf(),
                source,
            }
        })?;
    }
    Ok(())
}

fn write_new(path: &Path, bytes: &[u8], mode: u32) -> Result<(), PkiError> {
    if let Some(parent) = path.parent() {
        create_private_directory(parent)?;
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(mode);
    }
    let mut file = options.open(path).map_err(|source| PkiError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    file.write_all(bytes).map_err(|source| PkiError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    file.sync_all().map_err(|source| PkiError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn list(directory: &Path) -> Result<(), PkiError> {
    let authority_ready = directory.join("authority/ca.crt").is_file()
        && directory.join("authority/ca.key").is_file();
    let server_ready = directory.join("server/ca.crt").is_file()
        && directory.join("server/server.crt").is_file()
        && directory.join("server/server.key").is_file();
    println!(
        "Authority: {}",
        if authority_ready { "ready" } else { "missing" }
    );
    println!(
        "Server identity: {}",
        if server_ready { "ready" } else { "missing" }
    );
    print_identities("Clients", &directory.join("clients"))?;
    print_identities("Administrators", &directory.join("admins"))?;
    if server_ready {
        println!("Server bundle: {}", directory.join("server").display());
    }
    Ok(())
}

fn print_identities(label: &str, directory: &Path) -> Result<(), PkiError> {
    let mut names = if directory.is_dir() {
        let entries = fs::read_dir(directory).map_err(|source| PkiError::Io {
            path: directory.to_path_buf(),
            source,
        })?;
        let mut names = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|source| PkiError::Io {
                path: directory.to_path_buf(),
                source,
            })?;
            if entry
                .file_type()
                .map_err(|source| PkiError::Io {
                    path: entry.path(),
                    source,
                })?
                .is_dir()
                && let Ok(name) = entry.file_name().into_string()
            {
                names.push(name);
            }
        }
        names
    } else {
        Vec::new()
    };
    names.sort();
    println!(
        "{label}: {}",
        if names.is_empty() {
            "none".into()
        } else {
            names.join(", ")
        }
    );
    Ok(())
}

#[derive(Clone, Copy)]
enum ClientRole {
    Application,
    Administrator,
}

impl ClientRole {
    const fn bundle_directory(self) -> &'static str {
        match self {
            Self::Application => "clients",
            Self::Administrator => "admins",
        }
    }

    const fn certificate_name(self) -> &'static str {
        match self {
            Self::Application => "client.crt",
            Self::Administrator => "admin.crt",
        }
    }

    const fn key_name(self) -> &'static str {
        match self {
            Self::Application => "client.key",
            Self::Administrator => "admin.key",
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Application => "client",
            Self::Administrator => "administrator",
        }
    }

    const fn is_administrator(self) -> bool {
        matches!(self, Self::Administrator)
    }
}

#[derive(Debug, thiserror::Error)]
pub(super) enum PkiError {
    #[error("PKI is not initialized in {0}; run `openkache-server pki init` first")]
    NotInitialized(PathBuf),
    #[error("refusing to overwrite existing PKI artifact {0}")]
    AlreadyExists(PathBuf),
    #[error("identity name {0:?} must use 1-64 ASCII letters, digits, '.', '-', or '_'")]
    InvalidName(String),
    #[error("certificate validity must be at least one day and fit the supported time range")]
    InvalidValidity,
    #[error("certificate generation failed: {0}")]
    Certificate(#[from] rcgen::Error),
    #[error("PKI file {path} failed: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}
