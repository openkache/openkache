//! Node-API adapter for the OpenKache client on Node.js, Bun, and Deno.

use std::net::SocketAddr;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use napi::bindgen_prelude::Uint8Array;
use napi::{Error, Result, Status};
use napi_derive::napi;
use openkache_client_core::value::{Compression, ZstandardOptions};
use openkache_client_core::{
    Certificate, ClientIdentity, ClientTimeouts, DataProtectionKey, DeleteOutcome, Endpoint,
    GetOutcome, PrivateKey, ProtectedClient, SetCondition, SetOptions, SetOutcome, value_envelope,
};

const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

/// Mutual TLS identity accepted from JavaScript.
#[napi(object)]
pub struct NativeIdentity {
    #[napi(js_name = "certificate_chain")]
    pub certificate_chain: Vec<Uint8Array>,
    #[napi(js_name = "private_key")]
    pub private_key: Uint8Array,
}

/// Fully resolved connection and value transformation settings.
#[napi(object)]
pub struct NativeClientOptions {
    pub address: String,
    #[napi(js_name = "server_name")]
    pub server_name: String,
    pub certificate: Uint8Array,
    pub identity: Option<NativeIdentity>,
    #[napi(js_name = "data_protection_key")]
    pub data_protection_key: Uint8Array,
    #[napi(js_name = "compression_enabled")]
    pub compression_enabled: bool,
    #[napi(js_name = "compression_level")]
    pub compression_level: Option<i32>,
    #[napi(js_name = "minimum_input_size")]
    pub minimum_input_size: Option<f64>,
    #[napi(js_name = "minimum_savings")]
    pub minimum_savings: Option<f64>,
    #[napi(js_name = "connect_timeout_ms")]
    pub connect_timeout_ms: Option<f64>,
    #[napi(js_name = "request_timeout_ms")]
    pub request_timeout_ms: Option<f64>,
}

/// Decoded components of a canonical OpenKache value envelope.
#[napi(object)]
pub struct NativeValueEnvelope {
    pub encoding: String,
    #[napi(js_name = "type_name")]
    pub type_name: String,
    pub payload: Uint8Array,
}

/// Closable Node-API handle shared by Node.js, Bun, and Deno.
#[napi]
pub struct NativeClient {
    client: RwLock<Option<Arc<ProtectedClient>>>,
}

#[napi]
impl NativeClient {
    /// Verifies that the server is reachable.
    #[napi]
    pub async fn ping(&self) -> Result<()> {
        self.active_client()?
            .ping()
            .await
            .map(|_| ())
            .map_err(native_error)
    }

    /// Retrieves exact decoded bytes or `null` when the key is absent.
    #[napi]
    pub async fn get(&self, key: Uint8Array) -> Result<Option<Uint8Array>> {
        self.active_client()?
            .get(key.as_ref())
            .await
            .map(|value| value.into_option().map(Uint8Array::new))
            .map_err(native_error)
    }

    /// Retrieves and decodes a canonical value envelope.
    ///
    /// # Arguments
    ///
    /// * `key` - Exact application key bytes.
    ///
    /// # Returns
    ///
    /// Decoded codec metadata and payload, or `None` when the key is absent.
    ///
    /// # Errors
    ///
    /// Returns an error when the client is closed, transport or value transformation fails, or
    /// the stored bytes are not a supported value envelope.
    #[napi(js_name = "get_value")]
    pub async fn get_value(&self, key: Uint8Array) -> Result<Option<NativeValueEnvelope>> {
        let GetOutcome::Found(bytes) = self
            .active_client()?
            .get(key.as_ref())
            .await
            .map_err(native_error)?
        else {
            return Ok(None);
        };
        let envelope = value_envelope::decode(&bytes).map_err(native_error)?;
        Ok(Some(NativeValueEnvelope {
            encoding: envelope.encoding.to_owned(),
            type_name: envelope.type_name.to_owned(),
            payload: Uint8Array::new(envelope.payload.to_vec()),
        }))
    }

    /// Stores exact bytes with an optional existence condition and TTL.
    #[napi]
    pub async fn set(
        &self,
        key: Uint8Array,
        value: Uint8Array,
        condition: Option<String>,
        ttl_ms: Option<f64>,
    ) -> Result<String> {
        self.store(key.as_ref(), value.as_ref().to_vec(), condition, ttl_ms)
            .await
    }

    /// Encodes and stores a canonical value envelope.
    ///
    /// # Arguments
    ///
    /// * `key` - Exact application key bytes.
    /// * `encoding` - Portable codec identifier.
    /// * `type_name` - Codec-defined logical type name.
    /// * `payload` - Exact codec-specific bytes.
    /// * `condition` - Optional `if_absent` or `if_present` existence condition.
    /// * `ttl_ms` - Optional positive relative lifetime in milliseconds.
    ///
    /// # Returns
    ///
    /// The server's created, replaced, or not-stored outcome.
    ///
    /// # Errors
    ///
    /// Returns an error when envelope metadata or options are invalid, the value is too large,
    /// the client is closed, or the operation fails.
    #[napi(js_name = "set_value")]
    pub async fn set_value(
        &self,
        key: Uint8Array,
        encoding: String,
        type_name: String,
        payload: Uint8Array,
        condition: Option<String>,
        ttl_ms: Option<f64>,
    ) -> Result<String> {
        let value = value_envelope::encode(&encoding, &type_name, payload.as_ref())
            .map_err(native_error)?;
        self.store(key.as_ref(), value, condition, ttl_ms).await
    }

    /// Deletes a key and reports whether it existed.
    #[napi]
    pub async fn delete(&self, key: Uint8Array) -> Result<bool> {
        self.active_client()?
            .delete(key.as_ref())
            .await
            .map(|outcome| outcome == DeleteOutcome::Deleted)
            .map_err(native_error)
    }

    /// Returns the server's JSON statistics payload.
    #[napi]
    pub async fn stats(&self) -> Result<String> {
        self.active_client()?.stats().await.map_err(native_error)
    }

    /// Requests a server durability barrier.
    #[napi]
    pub async fn sync(&self) -> Result<()> {
        self.active_client()?.sync().await.map_err(native_error)
    }

    /// Releases the native client. Repeated calls are safe.
    #[napi]
    pub fn close(&self) -> Result<()> {
        self.client
            .write()
            .map_err(|_| state_error("native client state lock is poisoned"))?
            .take();
        Ok(())
    }
}

impl NativeClient {
    async fn store(
        &self,
        key: &[u8],
        value: Vec<u8>,
        condition: Option<String>,
        ttl_ms: Option<f64>,
    ) -> Result<String> {
        let condition = parse_condition(condition.as_deref())?;
        let ttl_ms = ttl_ms
            .map(|value| parse_u64(value, "ttl_ms", false))
            .transpose()?;
        let client = self.active_client()?;
        let mut options = match condition {
            SetCondition::None => SetOptions::new(),
            SetCondition::IfAbsent => SetOptions::new().if_absent(),
            SetCondition::IfPresent => SetOptions::new().if_present(),
        };
        if let Some(ttl_ms) = ttl_ms {
            options = options.expires_after_millis(ttl_ms);
        }
        client
            .set(key, value, options)
            .await
            .map(|outcome| match outcome {
                SetOutcome::Created => "created".to_string(),
                SetOutcome::Replaced => "replaced".to_string(),
                SetOutcome::NotStored => "not_stored".to_string(),
            })
            .map_err(native_error)
    }

    fn active_client(&self) -> Result<Arc<ProtectedClient>> {
        self.client
            .read()
            .map_err(|_| state_error("native client state lock is poisoned"))?
            .as_ref()
            .map(Arc::clone)
            .ok_or_else(|| state_error("client is closed"))
    }
}

/// Connects Node.js, Bun, or Deno using the shared Rust implementation.
///
/// # Errors
///
/// Returns an error for invalid options, certificate or key parsing failures, and connection
/// failures.
#[napi]
pub async fn connect(options: NativeClientOptions) -> Result<NativeClient> {
    let address: SocketAddr = options
        .address
        .parse()
        .map_err(|error| invalid_argument(format!("invalid server address: {error}")))?;
    let mut trusted_certificates =
        Certificate::from_der_or_pem_chain(options.certificate.as_ref()).map_err(native_error)?;
    if trusted_certificates.len() != 1 {
        return Err(invalid_argument(format!(
            "certificate must contain exactly one DER or PEM certificate, got {}",
            trusted_certificates.len()
        )));
    }

    let data_protection_key =
        DataProtectionKey::from_slice(options.data_protection_key.as_ref())
            .map_err(native_error)?;
    let compression = if options.compression_enabled {
        let defaults = ZstandardOptions::default();
        Compression::Zstandard(ZstandardOptions {
            level: options.compression_level.unwrap_or(defaults.level),
            minimum_input_size: options
                .minimum_input_size
                .map(|value| parse_usize(value, "minimum_input_size", true))
                .transpose()?
                .unwrap_or(defaults.minimum_input_size),
            minimum_savings: options
                .minimum_savings
                .map(|value| parse_usize(value, "minimum_savings", true))
                .transpose()?
                .unwrap_or(defaults.minimum_savings),
        })
    } else {
        Compression::Disabled
    };
    let identity = parse_identity(options.identity)?;
    let mut timeouts = ClientTimeouts::default();
    if let Some(connect_timeout_ms) = options.connect_timeout_ms {
        timeouts.connect = Duration::from_millis(parse_u64(
            connect_timeout_ms,
            "connect_timeout_ms",
            false,
        )?);
    }
    if let Some(request_timeout_ms) = options.request_timeout_ms {
        timeouts.request = Duration::from_millis(parse_u64(
            request_timeout_ms,
            "request_timeout_ms",
            false,
        )?);
    }

    let endpoint =
        Endpoint::from_socket_addr(address, options.server_name).map_err(native_error)?;
    let trusted_certificate = trusted_certificates.remove(0);
    let mut builder = ProtectedClient::builder(endpoint, data_protection_key)
        .trust_certificate(trusted_certificate)
        .compression(compression)
        .timeouts(timeouts);
    if let Some(identity) = identity {
        builder = builder.client_identity(identity);
    }
    let client = builder.connect().await.map_err(native_error)?;
    Ok(NativeClient {
        client: RwLock::new(Some(Arc::new(client))),
    })
}

fn parse_identity(identity: Option<NativeIdentity>) -> Result<Option<ClientIdentity>> {
    let Some(identity) = identity else {
        return Ok(None);
    };
    let mut certificate_chain = Vec::new();
    for certificate in identity.certificate_chain {
        certificate_chain.extend(
            Certificate::from_der_or_pem_chain(certificate.as_ref()).map_err(native_error)?,
        );
    }
    if certificate_chain.is_empty() {
        return Err(invalid_argument(
            "client certificate chain must not be empty",
        ));
    }
    let private_key = parse_private_key(identity.private_key.as_ref())?;
    ClientIdentity::new(certificate_chain, private_key)
        .map(Some)
        .map_err(native_error)
}

fn parse_private_key(bytes: &[u8]) -> Result<PrivateKey> {
    PrivateKey::from_der_or_pem(bytes).map_err(native_error)
}

fn parse_condition(condition: Option<&str>) -> Result<SetCondition> {
    match condition {
        None => Ok(SetCondition::None),
        Some("if_absent") => Ok(SetCondition::IfAbsent),
        Some("if_present") => Ok(SetCondition::IfPresent),
        Some(value) => Err(invalid_argument(format!(
            "condition must be if_absent or if_present, got {value}"
        ))),
    }
}

fn parse_usize(value: f64, name: &str, allow_zero: bool) -> Result<usize> {
    let value = parse_u64(value, name, allow_zero)?;
    usize::try_from(value)
        .map_err(|_| invalid_argument(format!("{name} exceeds the native platform limit")))
}

fn parse_u64(value: f64, name: &str, allow_zero: bool) -> Result<u64> {
    if !value.is_finite()
        || value.fract() != 0.0
        || value < if allow_zero { 0.0 } else { 1.0 }
        || value > MAX_SAFE_INTEGER
    {
        let requirement = if allow_zero {
            "a non-negative safe integer"
        } else {
            "a positive safe integer"
        };
        return Err(invalid_argument(format!("{name} must be {requirement}")));
    }
    Ok(value as u64)
}

fn native_error(error: impl std::fmt::Display) -> Error {
    Error::new(Status::GenericFailure, error.to_string())
}

fn invalid_argument(message: impl Into<String>) -> Error {
    Error::new(Status::InvalidArg, message.into())
}

fn state_error(message: impl Into<String>) -> Error {
    Error::new(Status::GenericFailure, message.into())
}
