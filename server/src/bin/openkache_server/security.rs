//! Security-related command-line arguments for the OpenKache server.

use std::path::{Path, PathBuf};

use anstream::println;
use clap::Args;
use openkache::AppConfig;
use openkache::server::{KacheServer, Result};
use owo_colors::OwoColorize;

/// TLS identity, optional client authentication, and explicit development
/// override arguments.
#[derive(Args)]
pub(super) struct SecurityArguments {
    /// File receiving the generated certificate in local development modes.
    #[arg(long, default_value = "target/openkache-local/certificate.local.der")]
    certificate_out: PathBuf,

    /// Allow insecure development, including on an explicitly selected non-loopback address.
    #[arg(
        long,
        conflicts_with_all = [
            "pki_directory",
            "tls_certificate_chain",
            "tls_private_key",
            "tls_client_ca",
            "tls_admin_client_certificates"
        ]
    )]
    insecure_development: bool,

    /// Deployable directory created by `openkache-server pki`.
    #[arg(
        long,
        value_name = "PATH",
        conflicts_with_all = [
            "insecure_development",
            "tls_certificate_chain",
            "tls_private_key",
            "tls_client_ca",
            "tls_admin_client_certificates"
        ]
    )]
    pki_directory: Option<PathBuf>,

    /// PEM or DER server certificate chain, with the leaf certificate first.
    #[arg(long, value_name = "PATH")]
    tls_certificate_chain: Option<PathBuf>,

    /// Unencrypted PEM or DER server private key.
    #[arg(long, value_name = "PATH")]
    tls_private_key: Option<PathBuf>,

    /// Optional PEM or DER CA certificates trusted to authenticate clients.
    /// Omit this flag when ordinary TLS clients do not need mTLS.
    #[arg(long, value_name = "PATH")]
    tls_client_ca: Option<PathBuf>,

    /// Authenticated client leaf certificate allowed to run EXPERIMENTAL_STATS
    /// and EXPERIMENTAL_SYNC; repeatable. Requires --tls-client-ca.
    #[arg(long = "tls-admin-client-certificate", value_name = "PATH")]
    tls_admin_client_certificates: Vec<PathBuf>,
}

impl SecurityArguments {
    pub(super) fn apply(&self, config: &mut AppConfig) -> std::io::Result<()> {
        if let Some(directory) = &self.pki_directory {
            return apply_pki_directory(config, directory);
        }
        if let Some(path) = &self.tls_certificate_chain {
            config.tls.certificate_chain = Some(path.clone());
        }
        if let Some(path) = &self.tls_private_key {
            config.tls.private_key = Some(path.clone());
        }
        if let Some(path) = &self.tls_client_ca {
            config.tls.client_ca = Some(path.clone());
        }
        if !self.tls_admin_client_certificates.is_empty() {
            config.tls.admin_client_certificates = self.tls_admin_client_certificates.clone();
        }
        Ok(())
    }

    pub(super) async fn bind(
        &self,
        listen: std::net::SocketAddr,
        config: AppConfig,
    ) -> Result<(KacheServer, SecurityMode)> {
        if self.insecure_development {
            KacheServer::bind_insecure_for_development(listen, config)
                .await
                .map(|server| (server, SecurityMode::ExplicitInsecureDevelopment))
        } else if local_development_by_default(listen, config.tls.is_configured()) {
            KacheServer::bind_insecure_for_development(listen, config)
                .await
                .map(|server| (server, SecurityMode::LocalDevelopment))
        } else {
            let client_authentication = config.tls.client_ca.is_some();
            KacheServer::bind_with_config(listen, config)
                .await
                .map(|server| {
                    (
                        server,
                        SecurityMode::ProductionTls {
                            client_authentication,
                        },
                    )
                })
        }
    }

    pub(super) fn write_development_certificate(
        &self,
        server: &KacheServer,
        mode: SecurityMode,
    ) -> std::io::Result<()> {
        if !mode.is_development() {
            return Ok(());
        }
        if let Some(parent) = self.certificate_out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&self.certificate_out, server.certificate_der())
    }

    pub(super) fn report(&self, mode: SecurityMode) {
        match mode {
            SecurityMode::ProductionTls {
                client_authentication: true,
            } => println!("{} {}", "Security:".green().bold(), "mutual TLS"),
            SecurityMode::ProductionTls {
                client_authentication: false,
            } => println!(
                "{} TLS 1.3 (server identity configured; client certificates optional)",
                "Security:".green().bold()
            ),
            SecurityMode::LocalDevelopment => println!(
                "{} TLS 1.3 local development on loopback only; client certificates omitted (server certificate: {})",
                "Security:".green().bold(),
                self.certificate_out.display()
            ),
            SecurityMode::ExplicitInsecureDevelopment => println!(
                "{} INSECURE DEVELOPMENT (TLS 1.3; client certificates omitted; server certificate: {})",
                "Security:".red().bold(),
                self.certificate_out.display()
            ),
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum SecurityMode {
    ProductionTls {
        client_authentication: bool,
    },
    LocalDevelopment,
    ExplicitInsecureDevelopment,
}

impl SecurityMode {
    const fn is_development(self) -> bool {
        matches!(
            self,
            Self::LocalDevelopment | Self::ExplicitInsecureDevelopment
        )
    }
}

pub(super) fn local_development_by_default(
    listen: std::net::SocketAddr,
    production_tls_configured: bool,
) -> bool {
    !production_tls_configured && listen.ip().is_loopback()
}

pub(super) fn apply_pki_directory(config: &mut AppConfig, directory: &Path) -> std::io::Result<()> {
    config.tls.certificate_chain = Some(directory.join("server.crt"));
    config.tls.private_key = Some(directory.join("server.key"));
    config.tls.client_ca = Some(directory.join("ca.crt"));
    config.tls.admin_client_certificates = administrator_certificates(&directory.join("admins"))?;
    Ok(())
}

pub(super) fn administrator_certificates(directory: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut certificates = Vec::new();
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|extension| extension == "crt")
            && entry.file_type()?.is_file()
        {
            certificates.push(path);
        }
    }
    certificates.sort();
    Ok(certificates)
}
