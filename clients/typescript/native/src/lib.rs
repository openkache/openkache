//! Node-API adapter for the OpenKache client on Node.js, Bun, and Deno.

use std::sync::{Arc, RwLock};
use std::time::Duration;

use napi::bindgen_prelude::{BigInt, Uint8Array};
use napi::{Error, Result, Status};
use napi_derive::napi;
use openkache_client_core::value::{Compression, Encryption, JsonValue, Value, ZstandardOptions};
use openkache_client_core::{
    Certificate, ClientIdentity, ClientTimeouts, DEFAULT_MAX_IN_FLIGHT, DataProtectionKey,
    DeleteOutcome, Endpoint, EvictionDefault, ExpirationDefault, GetOutcome, ItemId, ItemValue,
    KeyType, NamespaceDescriptor, NamespacePolicy, Opcode, OverridePolicy, PrivateKey,
    ProtectedClient, ResolvedKey, RetryPolicy, SetCondition, SetOptions, SetOutcome,
    contract::{
        ConnectionState, SMITHY_EVICTION_DEFAULT_EVICTABLE,
        SMITHY_EVICTION_DEFAULT_EVICTION_PROTECTED, SMITHY_EVICTION_MODE_EVICTABLE,
        SMITHY_EVICTION_MODE_EVICTION_PROTECTED, SMITHY_EVICTION_MODE_INHERIT,
        SMITHY_EXPIRATION_DEFAULT_FIXED_TTL, SMITHY_EXPIRATION_DEFAULT_NO_EXPIRY,
        SMITHY_EXPIRATION_MODE_EXPLICIT_TTL, SMITHY_EXPIRATION_MODE_INHERIT,
        SMITHY_EXPIRATION_MODE_NO_EXPIRY, SMITHY_OVERRIDE_POLICY_ALLOWED,
        SMITHY_OVERRIDE_POLICY_DISALLOWED, SMITHY_SET_CONDITION_ANY,
        SMITHY_SET_CONDITION_IF_ABSENT, SMITHY_SET_CONDITION_IF_PRESENT,
        SMITHY_SET_OUTCOME_CREATED, SMITHY_SET_OUTCOME_NOT_STORED, SMITHY_SET_OUTCOME_REPLACED,
    },
    value_envelope,
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
    #[napi(js_name = "retry_max_attempts")]
    pub retry_max_attempts: Option<f64>,
    #[napi(js_name = "max_in_flight")]
    pub max_in_flight: Option<f64>,
    pub encryption: Option<String>,
    #[napi(js_name = "key_type")]
    pub key_type: Option<String>,
}

/// Decoded components of a canonical OpenKache value envelope.
#[napi(object)]
pub struct NativeValueEnvelope {
    pub encoding: String,
    #[napi(js_name = "type_name")]
    pub type_name: String,
    pub payload: Uint8Array,
}

/// Namespace policy represented with Smithy enum strings and lossless JavaScript BigInts.
#[napi(object)]
pub struct NativeNamespacePolicy {
    #[napi(js_name = "default_expiration")]
    pub default_expiration: String,
    #[napi(js_name = "default_ttl_milliseconds")]
    pub default_ttl_milliseconds: Option<BigInt>,
    #[napi(js_name = "expiration_override")]
    pub expiration_override: String,
    #[napi(js_name = "default_eviction")]
    pub default_eviction: String,
    #[napi(js_name = "eviction_override")]
    pub eviction_override: String,
}

/// Namespace identity and policy returned by namespace-management operations.
#[napi(object)]
pub struct NativeNamespaceDescriptor {
    #[napi(js_name = "namespace_id")]
    pub namespace_id: BigInt,
    pub revision: BigInt,
    pub policy: NativeNamespacePolicy,
}

/// Result returned by NAMESPACE_OPEN.
#[napi(object)]
pub struct NativeNamespaceOpenOutput {
    pub descriptor: NativeNamespaceDescriptor,
    pub created: bool,
}

/// Result returned by a generated Smithy operation invocation.
#[napi(object)]
pub struct NativeOperationResult {
    pub kind: u32,
    pub status: u32,
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

    /// Executes a generated Smithy operation against exact item-ID storage.
    #[napi(js_name = "execute_raw")]
    pub async fn execute_raw(
        &self,
        operation: u32,
        item_id: Uint8Array,
        value: Uint8Array,
        condition: Option<String>,
        expiration_mode: Option<String>,
        eviction_mode: Option<String>,
        ttl_ms: Option<BigInt>,
    ) -> Result<NativeOperationResult> {
        let opcode = parse_opcode(operation)?;
        let options = parse_wire_set_options(
            condition.as_deref(),
            expiration_mode.as_deref(),
            eviction_mode.as_deref(),
            ttl_ms,
        )?;
        self.active_client()?
            .raw()
            .execute_raw(opcode, item_id.as_ref(), value.as_ref(), options)
            .await
            .map(native_operation_result)
            .map_err(native_error)
    }

    /// Executes a generated Smithy operation in an explicitly supplied namespace.
    #[napi(js_name = "execute_scoped")]
    pub async fn execute_scoped(
        &self,
        operation: u32,
        namespace_id: BigInt,
        item_id: Uint8Array,
        value: Uint8Array,
        condition: Option<String>,
        expiration_mode: Option<String>,
        eviction_mode: Option<String>,
        ttl_ms: Option<BigInt>,
    ) -> Result<NativeOperationResult> {
        let opcode = parse_opcode(operation)?;
        let namespace_id = parse_bigint_u64(namespace_id, "namespace_id", false)?;
        let options = parse_wire_set_options(
            condition.as_deref(),
            expiration_mode.as_deref(),
            eviction_mode.as_deref(),
            ttl_ms,
        )?;
        self.active_client()?
            .raw()
            .execute_scoped(
                opcode,
                namespace_id,
                item_id.as_ref(),
                value.as_ref(),
                options,
            )
            .await
            .map(native_operation_result)
            .map_err(native_error)
    }

    /// Retrieves exact decoded bytes or `null` when the key is absent.
    #[napi]
    pub async fn get(&self, key: Uint8Array) -> Result<Option<Uint8Array>> {
        let key = self.logical_key(key.as_ref())?;
        self.active_client()?
            .get_resolved(key)
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
        let key = self.logical_key(key.as_ref())?;
        let GetOutcome::Found(bytes) = self
            .active_client()?
            .get_resolved(key)
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

    /// Retrieves a core-owned canonical JSON value.
    ///
    /// Raw-formatted values are rejected instead of being silently coerced.
    #[napi(js_name = "get_json")]
    pub async fn get_json(&self, key: Uint8Array) -> Result<Option<String>> {
        let key = self.logical_key(key.as_ref())?;
        let outcome = self
            .active_client()?
            .get_value_resolved(key)
            .await
            .map_err(native_error)?;
        match outcome {
            GetOutcome::NotFound => Ok(None),
            GetOutcome::Found(Value::Json(value)) => serde_json::to_string(&value)
                .map(Some)
                .map_err(native_error),
            GetOutcome::Found(Value::Raw(_)) => Err(native_error(
                "stored value uses raw serialization, expected canonical JSON",
            )),
        }
    }

    /// Stores exact bytes with an optional existence condition and TTL.
    #[napi]
    pub async fn set(
        &self,
        key: Uint8Array,
        value: Uint8Array,
        condition: Option<String>,
        expiration_mode: Option<String>,
        eviction_mode: Option<String>,
        ttl_ms: Option<f64>,
    ) -> Result<String> {
        self.store(
            key.as_ref(),
            value.as_ref().to_vec(),
            condition,
            expiration_mode,
            eviction_mode,
            ttl_ms,
        )
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
        expiration_mode: Option<String>,
        eviction_mode: Option<String>,
        ttl_ms: Option<f64>,
    ) -> Result<String> {
        let value = value_envelope::encode(&encoding, &type_name, payload.as_ref())
            .map_err(native_error)?;
        self.store(
            key.as_ref(),
            value,
            condition,
            expiration_mode,
            eviction_mode,
            ttl_ms,
        )
        .await
    }

    /// Serializes and stores a core-owned canonical JSON value.
    #[napi(js_name = "set_json")]
    pub async fn set_json(
        &self,
        key: Uint8Array,
        value: serde_json::Value,
        condition: Option<String>,
        expiration_mode: Option<String>,
        eviction_mode: Option<String>,
        ttl_ms: Option<f64>,
    ) -> Result<String> {
        let value = parse_json_value(value)?;
        let options = parse_set_options(
            condition.as_deref(),
            expiration_mode.as_deref(),
            eviction_mode.as_deref(),
            ttl_ms,
        )?;
        let key = self.logical_key(key.as_ref())?;
        self.active_client()?
            .set_value_resolved(key, Value::Json(value), options)
            .await
            .map(map_set_outcome)
            .map_err(native_error)
    }

    /// Deletes a key and reports whether it existed.
    #[napi]
    pub async fn delete(&self, key: Uint8Array) -> Result<bool> {
        let key = self.logical_key(key.as_ref())?;
        self.active_client()?
            .delete_resolved(key)
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

    /// Closes the shared core client. Repeated calls are safe.
    #[napi]
    pub async fn close(&self) -> Result<()> {
        let client = self.take_client()?;
        if let Some(client) = client {
            client.close().await.map_err(native_error)?;
        }
        Ok(())
    }

    /// Drops the native handle without awaiting the core shutdown future.
    ///
    /// This is reserved for a JavaScript finalizer, where no promise can be observed.
    #[napi(js_name = "close_now")]
    pub fn close_now(&self) -> Result<()> {
        self.take_client()?;
        Ok(())
    }

    /// Returns the shared core's best-effort connection state.
    #[napi(js_name = "connection_state")]
    pub fn connection_state(&self) -> Result<String> {
        let client = self
            .client
            .read()
            .map_err(|_| state_error("native client state lock is poisoned"))?;
        Ok(client
            .as_ref()
            .map(|client| client.connection_state().to_string())
            .unwrap_or_else(|| ConnectionState::Closed.to_string()))
    }

    /// Reconnects the shared core without replaying a request.
    #[napi]
    pub async fn reconnect(&self) -> Result<()> {
        self.active_client()?
            .reconnect()
            .await
            .map_err(native_error)
    }

    /// Retrieves exact bytes for an opaque protocol item ID.
    #[napi(js_name = "raw_get")]
    pub async fn raw_get(&self, item_id: Uint8Array) -> Result<Option<Uint8Array>> {
        let item_id = parse_item_id(item_id.as_ref())?;
        self.active_client()?
            .raw()
            .get(item_id)
            .await
            .map(|value| {
                value
                    .into_option()
                    .map(|value| Uint8Array::new(value.into_bytes()))
            })
            .map_err(native_error)
    }

    /// Retrieves exact bytes in an explicitly supplied namespace.
    #[napi(js_name = "raw_get_in_namespace")]
    pub async fn raw_get_in_namespace(
        &self,
        namespace_id: BigInt,
        item_id: Uint8Array,
    ) -> Result<Option<Uint8Array>> {
        let namespace_id = parse_bigint_u64(namespace_id, "namespace_id", false)?;
        let item_id = parse_item_id(item_id.as_ref())?;
        self.active_client()?
            .raw()
            .get_in_namespace(namespace_id, item_id)
            .await
            .map(|value| {
                value
                    .into_option()
                    .map(|value| Uint8Array::new(value.into_bytes()))
            })
            .map_err(native_error)
    }

    /// Stores exact bytes for an opaque protocol item ID.
    #[napi(js_name = "raw_set")]
    pub async fn raw_set(
        &self,
        item_id: Uint8Array,
        value: Uint8Array,
        condition: Option<String>,
        expiration_mode: Option<String>,
        eviction_mode: Option<String>,
        ttl_ms: Option<f64>,
    ) -> Result<String> {
        let item_id = parse_item_id(item_id.as_ref())?;
        let options = parse_set_options(
            condition.as_deref(),
            expiration_mode.as_deref(),
            eviction_mode.as_deref(),
            ttl_ms,
        )?;
        self.active_client()?
            .raw()
            .set(item_id, ItemValue::new(value.as_ref().to_vec()), options)
            .await
            .map(map_set_outcome)
            .map_err(native_error)
    }

    /// Stores exact bytes with all item-level policy selectors in an explicit namespace.
    #[napi(js_name = "raw_set_in_namespace")]
    pub async fn raw_set_in_namespace(
        &self,
        namespace_id: BigInt,
        item_id: Uint8Array,
        value: Uint8Array,
        condition: Option<String>,
        expiration_mode: Option<String>,
        eviction_mode: Option<String>,
        ttl_ms: Option<BigInt>,
    ) -> Result<String> {
        let namespace_id = parse_bigint_u64(namespace_id, "namespace_id", false)?;
        let item_id = parse_item_id(item_id.as_ref())?;
        let options = parse_wire_set_options(
            condition.as_deref(),
            expiration_mode.as_deref(),
            eviction_mode.as_deref(),
            ttl_ms,
        )?;
        self.active_client()?
            .raw()
            .set_in_namespace(
                namespace_id,
                item_id,
                ItemValue::new(value.as_ref().to_vec()),
                options,
            )
            .await
            .map(map_set_outcome)
            .map_err(native_error)
    }

    /// Deletes an opaque protocol item ID.
    #[napi(js_name = "raw_delete")]
    pub async fn raw_delete(&self, item_id: Uint8Array) -> Result<bool> {
        let item_id = parse_item_id(item_id.as_ref())?;
        self.active_client()?
            .raw()
            .delete(item_id)
            .await
            .map(|outcome| outcome == DeleteOutcome::Deleted)
            .map_err(native_error)
    }

    /// Deletes an item ID in an explicitly supplied namespace.
    #[napi(js_name = "raw_delete_in_namespace")]
    pub async fn raw_delete_in_namespace(
        &self,
        namespace_id: BigInt,
        item_id: Uint8Array,
    ) -> Result<bool> {
        let namespace_id = parse_bigint_u64(namespace_id, "namespace_id", false)?;
        let item_id = parse_item_id(item_id.as_ref())?;
        self.active_client()?
            .raw()
            .delete_in_namespace(namespace_id, item_id)
            .await
            .map(|outcome| outcome == DeleteOutcome::Deleted)
            .map_err(native_error)
    }

    /// Retrieves a namespace by name and optionally creates it.
    #[napi(js_name = "namespace_open")]
    pub async fn namespace_open(
        &self,
        name: String,
        create_if_missing: bool,
        policy: Option<NativeNamespacePolicy>,
    ) -> Result<NativeNamespaceOpenOutput> {
        let policy = policy.map(parse_namespace_policy).transpose()?;
        let (descriptor, created) = self
            .active_client()?
            .raw()
            .namespace_open_with_outcome(name.as_bytes(), create_if_missing, policy)
            .await
            .map_err(native_error)?;
        Ok(NativeNamespaceOpenOutput {
            descriptor: native_namespace_descriptor(descriptor),
            created,
        })
    }

    /// Replaces a namespace policy using its current revision.
    #[napi(js_name = "namespace_update_policy")]
    pub async fn namespace_update_policy(
        &self,
        namespace_id: BigInt,
        expected_revision: BigInt,
        policy: NativeNamespacePolicy,
    ) -> Result<NativeNamespaceDescriptor> {
        let namespace_id = parse_bigint_u64(namespace_id, "namespace_id", false)?;
        let expected_revision = parse_bigint_u64(expected_revision, "expected_revision", false)?;
        let policy = parse_namespace_policy(policy)?;
        let descriptor = self
            .active_client()?
            .raw()
            .namespace_update_policy(namespace_id, expected_revision, policy)
            .await
            .map_err(native_error)?;
        Ok(native_namespace_descriptor(descriptor))
    }

    /// Deletes an empty namespace using its current revision.
    #[napi(js_name = "namespace_delete")]
    pub async fn namespace_delete(
        &self,
        namespace_id: BigInt,
        expected_revision: BigInt,
    ) -> Result<()> {
        let namespace_id = parse_bigint_u64(namespace_id, "namespace_id", false)?;
        let expected_revision = parse_bigint_u64(expected_revision, "expected_revision", false)?;
        self.active_client()?
            .raw()
            .namespace_delete(namespace_id, expected_revision)
            .await
            .map_err(native_error)
    }

    /// Retrieves statistics for an explicitly supplied namespace.
    #[napi(js_name = "stats_in_namespace")]
    pub async fn stats_in_namespace(&self, namespace_id: BigInt) -> Result<String> {
        let namespace_id = parse_bigint_u64(namespace_id, "namespace_id", false)?;
        self.active_client()?
            .raw()
            .stats_in_namespace(namespace_id)
            .await
            .map_err(native_error)
    }

    /// Waits for a durability barrier in an explicitly supplied namespace.
    #[napi(js_name = "sync_in_namespace")]
    pub async fn sync_in_namespace(&self, namespace_id: BigInt) -> Result<()> {
        let namespace_id = parse_bigint_u64(namespace_id, "namespace_id", false)?;
        self.active_client()?
            .raw()
            .sync_in_namespace(namespace_id)
            .await
            .map_err(native_error)
    }
}

impl NativeClient {
    fn logical_key(&self, bytes: &[u8]) -> Result<ResolvedKey> {
        self.active_client()?
            .resolve_logical_key(bytes)
            .map_err(native_error)
    }

    async fn store(
        &self,
        key: &[u8],
        value: Vec<u8>,
        condition: Option<String>,
        expiration_mode: Option<String>,
        eviction_mode: Option<String>,
        ttl_ms: Option<f64>,
    ) -> Result<String> {
        let options = parse_set_options(
            condition.as_deref(),
            expiration_mode.as_deref(),
            eviction_mode.as_deref(),
            ttl_ms,
        )?;
        let client = self.active_client()?;
        let key = self.logical_key(key)?;
        client
            .set_resolved(key, value, options)
            .await
            .map(map_set_outcome)
            .map_err(native_error)
    }

    fn take_client(&self) -> Result<Option<Arc<ProtectedClient>>> {
        Ok(self
            .client
            .write()
            .map_err(|_| state_error("native client state lock is poisoned"))?
            .take())
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
    let mut trusted_certificates =
        Certificate::from_der_or_pem_chain(options.certificate.as_ref()).map_err(native_error)?;
    if trusted_certificates.len() != 1 {
        return Err(invalid_argument(format!(
            "certificate must contain exactly one DER or PEM certificate, got {}",
            trusted_certificates.len()
        )));
    }

    let data_protection_key = DataProtectionKey::from_slice(options.data_protection_key.as_ref())
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
        timeouts.connect =
            Duration::from_millis(parse_u64(connect_timeout_ms, "connect_timeout_ms", false)?);
    }
    if let Some(request_timeout_ms) = options.request_timeout_ms {
        timeouts.request =
            Duration::from_millis(parse_u64(request_timeout_ms, "request_timeout_ms", false)?);
    }

    let retry = RetryPolicy {
        max_attempts: options
            .retry_max_attempts
            .map(|value| parse_usize(value, "retry_max_attempts", false))
            .transpose()?
            .unwrap_or(RetryPolicy::default().max_attempts),
    };
    let max_in_flight = options
        .max_in_flight
        .map(|value| parse_usize(value, "max_in_flight", false))
        .transpose()?
        .unwrap_or(DEFAULT_MAX_IN_FLIGHT);
    let encryption = parse_encryption(options.encryption.as_deref())?;
    let key_type = parse_key_type(options.key_type.as_deref())?;
    let endpoint = parse_endpoint(&options.address, &options.server_name)?;
    let trusted_certificate = trusted_certificates.remove(0);
    let mut builder = ProtectedClient::builder(endpoint, data_protection_key)
        .trust_certificate(trusted_certificate)
        .compression(compression)
        .timeouts(timeouts)
        .retry_policy(retry)
        .max_in_flight(max_in_flight)
        .encryption(encryption)
        .key_type(key_type);
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

fn parse_bigint_u64(value: BigInt, name: &str, allow_zero: bool) -> Result<u64> {
    let (negative, value, lossless) = value.get_u64();
    if negative || !lossless || (!allow_zero && value == 0) {
        return Err(invalid_argument(format!(
            "{name} must be a positive unsigned 64-bit integer"
        )));
    }
    Ok(value)
}

fn parse_opcode(operation: u32) -> Result<Opcode> {
    let operation =
        u8::try_from(operation).map_err(|_| native_error("protocol opcode exceeds one byte"))?;
    Opcode::try_from(operation).map_err(native_error)
}

fn native_operation_result(
    result: openkache_client_core::OperationResult,
) -> NativeOperationResult {
    NativeOperationResult {
        kind: result.kind,
        status: result.status,
        payload: Uint8Array::new(result.payload),
    }
}

fn bigint_u64(value: u64) -> BigInt {
    BigInt {
        sign_bit: false,
        words: vec![value],
    }
}

fn parse_namespace_policy(policy: NativeNamespacePolicy) -> Result<NamespacePolicy> {
    let default_expiration = match policy.default_expiration.as_str() {
        value if value == SMITHY_EXPIRATION_DEFAULT_NO_EXPIRY => {
            if policy.default_ttl_milliseconds.is_some() {
                return Err(invalid_argument(format!(
                    "default_ttl_milliseconds is only valid with {} expiration",
                    SMITHY_EXPIRATION_DEFAULT_FIXED_TTL,
                )));
            }
            ExpirationDefault::NoExpiry
        }
        value if value == SMITHY_EXPIRATION_DEFAULT_FIXED_TTL => {
            let ttl_ms = policy.default_ttl_milliseconds.ok_or_else(|| {
                invalid_argument(format!(
                    "{} requires default_ttl_milliseconds",
                    SMITHY_EXPIRATION_DEFAULT_FIXED_TTL,
                ))
            })?;
            let ttl_ms = parse_bigint_u64(ttl_ms, "default_ttl_milliseconds", false)?;
            ExpirationDefault::FixedTtl { ttl_ms }
        }
        value => {
            return Err(invalid_argument(format!(
                "default_expiration must be {} or {}, got {value}",
                SMITHY_EXPIRATION_DEFAULT_NO_EXPIRY, SMITHY_EXPIRATION_DEFAULT_FIXED_TTL,
            )));
        }
    };
    let expiration_override = parse_override_policy(&policy.expiration_override)?;
    let default_eviction = match policy.default_eviction.as_str() {
        value if value == SMITHY_EVICTION_DEFAULT_EVICTABLE => EvictionDefault::Evictable,
        value if value == SMITHY_EVICTION_DEFAULT_EVICTION_PROTECTED => {
            EvictionDefault::EvictionProtected
        }
        value => {
            return Err(invalid_argument(format!(
                "default_eviction must be {} or {}, got {value}",
                SMITHY_EVICTION_DEFAULT_EVICTABLE, SMITHY_EVICTION_DEFAULT_EVICTION_PROTECTED,
            )));
        }
    };
    let eviction_override = parse_override_policy(&policy.eviction_override)?;
    Ok(NamespacePolicy {
        default_expiration,
        expiration_override,
        default_eviction,
        eviction_override,
    })
}

fn parse_override_policy(value: &str) -> Result<OverridePolicy> {
    match value {
        value if value == SMITHY_OVERRIDE_POLICY_ALLOWED => Ok(OverridePolicy::Allowed),
        value if value == SMITHY_OVERRIDE_POLICY_DISALLOWED => Ok(OverridePolicy::Disallowed),
        value => Err(invalid_argument(format!(
            "override policy must be {} or {}, got {value}",
            SMITHY_OVERRIDE_POLICY_ALLOWED, SMITHY_OVERRIDE_POLICY_DISALLOWED,
        ))),
    }
}

fn native_namespace_descriptor(descriptor: NamespaceDescriptor) -> NativeNamespaceDescriptor {
    let (default_expiration, default_ttl_milliseconds) = match descriptor.policy.default_expiration
    {
        ExpirationDefault::NoExpiry => (SMITHY_EXPIRATION_DEFAULT_NO_EXPIRY.to_owned(), None),
        ExpirationDefault::FixedTtl { ttl_ms } => (
            SMITHY_EXPIRATION_DEFAULT_FIXED_TTL.to_owned(),
            Some(bigint_u64(ttl_ms)),
        ),
    };
    NativeNamespaceDescriptor {
        namespace_id: bigint_u64(descriptor.namespace_id),
        revision: bigint_u64(descriptor.revision),
        policy: NativeNamespacePolicy {
            default_expiration,
            default_ttl_milliseconds,
            expiration_override: override_policy_string(descriptor.policy.expiration_override),
            default_eviction: match descriptor.policy.default_eviction {
                EvictionDefault::Evictable => SMITHY_EVICTION_DEFAULT_EVICTABLE.to_owned(),
                EvictionDefault::EvictionProtected => {
                    SMITHY_EVICTION_DEFAULT_EVICTION_PROTECTED.to_owned()
                }
            },
            eviction_override: override_policy_string(descriptor.policy.eviction_override),
        },
    }
}

fn override_policy_string(policy: OverridePolicy) -> String {
    match policy {
        OverridePolicy::Allowed => SMITHY_OVERRIDE_POLICY_ALLOWED,
        OverridePolicy::Disallowed => SMITHY_OVERRIDE_POLICY_DISALLOWED,
    }
    .to_owned()
}

fn parse_condition(condition: Option<&str>) -> Result<SetCondition> {
    match condition {
        None => Ok(SetCondition::Any),
        Some(value) if value == SMITHY_SET_CONDITION_ANY => Ok(SetCondition::Any),
        Some(value) if value == SMITHY_SET_CONDITION_IF_ABSENT => Ok(SetCondition::IfAbsent),
        Some(value) if value == SMITHY_SET_CONDITION_IF_PRESENT => Ok(SetCondition::IfPresent),
        Some(value) => Err(invalid_argument(format!(
            "condition must be {}, {}, or {}, got {value}",
            SMITHY_SET_CONDITION_ANY,
            SMITHY_SET_CONDITION_IF_ABSENT,
            SMITHY_SET_CONDITION_IF_PRESENT,
        ))),
    }
}

fn parse_set_options(
    condition: Option<&str>,
    expiration_mode: Option<&str>,
    eviction_mode: Option<&str>,
    ttl_ms: Option<f64>,
) -> Result<SetOptions> {
    let ttl_ms = ttl_ms
        .map(|value| parse_u64(value, "ttl_ms", false))
        .transpose()?;
    let expiration_mode = expiration_mode.or(if ttl_ms.is_some() {
        Some(SMITHY_EXPIRATION_MODE_EXPLICIT_TTL)
    } else {
        None
    });
    parse_wire_set_options(
        condition,
        expiration_mode,
        eviction_mode,
        ttl_ms.map(bigint_u64),
    )
}

fn parse_wire_set_options(
    condition: Option<&str>,
    expiration_mode: Option<&str>,
    eviction_mode: Option<&str>,
    ttl_ms: Option<BigInt>,
) -> Result<SetOptions> {
    let condition = parse_condition(condition)?;
    let mut options = match condition {
        SetCondition::Any => SetOptions::new(),
        SetCondition::IfAbsent => SetOptions::new().if_absent(),
        SetCondition::IfPresent => SetOptions::new().if_present(),
    };
    let expiration_mode = expiration_mode.unwrap_or(SMITHY_EXPIRATION_MODE_INHERIT);
    match expiration_mode {
        value if value == SMITHY_EXPIRATION_MODE_INHERIT => {
            if ttl_ms.is_some() {
                return Err(invalid_argument(format!(
                    "ttl_milliseconds is only valid with {} expiration",
                    SMITHY_EXPIRATION_MODE_EXPLICIT_TTL,
                )));
            }
            options = options.inherit_expiration();
        }
        value if value == SMITHY_EXPIRATION_MODE_NO_EXPIRY => {
            if ttl_ms.is_some() {
                return Err(invalid_argument(format!(
                    "ttl_milliseconds is only valid with {} expiration",
                    SMITHY_EXPIRATION_MODE_EXPLICIT_TTL,
                )));
            }
            options = options.no_expiry();
        }
        value if value == SMITHY_EXPIRATION_MODE_EXPLICIT_TTL => {
            let ttl_ms = ttl_ms.ok_or_else(|| {
                invalid_argument(format!(
                    "{} requires ttl_milliseconds",
                    SMITHY_EXPIRATION_MODE_EXPLICIT_TTL,
                ))
            })?;
            let ttl_ms = parse_bigint_u64(ttl_ms, "ttl_milliseconds", false)?;
            options = options.expires_after_millis(ttl_ms);
        }
        value => {
            return Err(invalid_argument(format!(
                "expiration_mode must be {}, {}, or {}, got {value}",
                SMITHY_EXPIRATION_MODE_INHERIT,
                SMITHY_EXPIRATION_MODE_NO_EXPIRY,
                SMITHY_EXPIRATION_MODE_EXPLICIT_TTL,
            )));
        }
    }
    match eviction_mode.unwrap_or(SMITHY_EVICTION_MODE_INHERIT) {
        value if value == SMITHY_EVICTION_MODE_INHERIT => {}
        value if value == SMITHY_EVICTION_MODE_EVICTABLE => options = options.evictable(),
        value if value == SMITHY_EVICTION_MODE_EVICTION_PROTECTED => {
            options = options.eviction_protected()
        }
        value => {
            return Err(invalid_argument(format!(
                "eviction_mode must be {}, {}, or {}, got {value}",
                SMITHY_EVICTION_MODE_INHERIT,
                SMITHY_EVICTION_MODE_EVICTABLE,
                SMITHY_EVICTION_MODE_EVICTION_PROTECTED,
            )));
        }
    }
    Ok(options)
}

fn parse_item_id(bytes: &[u8]) -> Result<ItemId> {
    ItemId::from_slice(bytes).map_err(native_error)
}

fn parse_json_value(value: serde_json::Value) -> Result<JsonValue> {
    match value {
        serde_json::Value::Null => Ok(JsonValue::Null),
        serde_json::Value::Bool(value) => Ok(JsonValue::Boolean(value)),
        serde_json::Value::Number(value) => parse_json_number(value),
        serde_json::Value::String(value) => Ok(JsonValue::String(value)),
        serde_json::Value::Array(values) => values
            .into_iter()
            .map(parse_json_value)
            .collect::<Result<Vec<_>>>()
            .map(JsonValue::Array),
        serde_json::Value::Object(values) => values
            .into_iter()
            .map(|(key, value)| parse_json_value(value).map(|value| (key, value)))
            .collect::<Result<Vec<_>>>()
            .map(JsonValue::Object),
    }
}

fn parse_json_number(value: serde_json::Number) -> Result<JsonValue> {
    let number = value
        .as_f64()
        .ok_or_else(|| invalid_argument("JSON number must fit in finite f64"))?;
    if let Some(integer) = value.as_i64() {
        validate_exact_json_integer(integer.unsigned_abs() as u128, number)?;
    } else if let Some(integer) = value.as_u64() {
        validate_exact_json_integer(integer as u128, number)?;
    }
    JsonValue::number(number).map_err(native_error)
}

fn validate_exact_json_integer(magnitude: u128, value: f64) -> Result<()> {
    let bit_length = u128::BITS - magnitude.leading_zeros();
    let exactly_representable =
        bit_length <= 53 || (magnitude & ((1_u128 << (bit_length - 53)) - 1) == 0);
    if !exactly_representable {
        return Err(invalid_argument(
            "JSON integers must be exactly representable as IEEE-754 binary64 values",
        ));
    }
    if !value.is_finite() {
        return Err(invalid_argument("JSON number must fit in finite f64"));
    }
    Ok(())
}

fn parse_endpoint(address: &str, server_name: &str) -> Result<Endpoint> {
    let address = address.parse().map_err(|error| {
        invalid_argument(format!("invalid server address {address:?}: {error}"))
    })?;
    Endpoint::from_socket_addr(address, server_name).map_err(native_error)
}

fn parse_encryption(encryption: Option<&str>) -> Result<Encryption> {
    match encryption {
        None | Some("robust") => Ok(Encryption::Robust),
        Some("compact") => Ok(Encryption::Compact),
        Some(value) => Err(invalid_argument(format!(
            "encryption must be compact or robust, got {value}"
        ))),
    }
}

fn parse_key_type(key_type: Option<&str>) -> Result<KeyType> {
    match key_type {
        None => Ok(KeyType::Text),
        Some(value) => KeyType::from_name(value).ok_or_else(|| {
            invalid_argument(format!(
                "key_type must be integer, text, or bytes, got {value}"
            ))
        }),
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

fn map_set_outcome(outcome: SetOutcome) -> String {
    match outcome {
        SetOutcome::Created => SMITHY_SET_OUTCOME_CREATED.to_owned(),
        SetOutcome::Replaced => SMITHY_SET_OUTCOME_REPLACED.to_owned(),
        SetOutcome::NotStored => SMITHY_SET_OUTCOME_NOT_STORED.to_owned(),
    }
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
