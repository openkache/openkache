//! Node-API adapter for the OpenKache client on Node.js, Bun, and Deno.

use std::collections::{HashMap, hash_map::Entry};
use std::future::Future;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use futures_util::future::{AbortHandle, Abortable};
use napi::bindgen_prelude::Uint8Array;
use napi::{Error, Result, Status};
use napi_derive::napi;
use openkache_client_core::contract::{
    FFI_ERROR_AMBIGUOUS, FFI_ERROR_CANCELLED, FFI_ERROR_CLOSED, FFI_ERROR_CONFIGURATION,
    FFI_ERROR_CONNECTION, FFI_ERROR_IO, FFI_ERROR_PROTOCOL, FFI_ERROR_RESPONSE_TOO_LARGE,
    FFI_ERROR_RUNTIME, FFI_ERROR_SERVER, FFI_ERROR_TIMEOUT, FFI_ERROR_TLS, FFI_ERROR_TRANSPORT,
    FFI_ERROR_UNEXPECTED_RESPONSE, FFI_ERROR_VALUE, FfiOperation,
};
use openkache_client_core::value::{
    Compression, Encryption, JsonValue, Value, ZstandardOptions, canonical_json_bytes,
};
use openkache_client_core::{
    Certificate, ClientIdentity, ClientTimeouts, DEFAULT_MAX_IN_FLIGHT, DataProtectionKey,
    DataProtectionKeyRing, DeleteOutcome, Endpoint, Error as CoreError, GetOutcome, ItemId,
    ItemValue, MAX_PREVIOUS_DATA_PROTECTION_KEYS, MutationId, PrivateKey, ProtectedClient,
    RetryPolicy, ServerErrorCode, SetCondition, SetOptions, SetOutcome, random_mutation_id,
};

const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;
const MAX_SAFE_REQUEST_ID: u64 = 9_007_199_254_740_991;

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
    #[napi(js_name = "previous_data_protection_keys")]
    pub previous_data_protection_keys: Option<Vec<Uint8Array>>,
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
}

/// Point-in-time native request and transport counters.
#[napi(object)]
pub struct NativeMetricsSnapshot {
    pub requests: f64,
    pub hits: f64,
    pub misses: f64,
    pub retries: f64,
    pub reconnects: f64,
    pub cancellations: f64,
    #[napi(js_name = "transport_errors")]
    pub transport_errors: f64,
    #[napi(js_name = "protocol_errors")]
    pub protocol_errors: f64,
    #[napi(js_name = "bytes_sent")]
    pub bytes_sent: f64,
    #[napi(js_name = "bytes_received")]
    pub bytes_received: f64,
    #[napi(js_name = "active_lanes")]
    pub active_lanes: f64,
}

#[derive(Default)]
struct LocalMetrics {
    requests: AtomicU64,
    hits: AtomicU64,
    misses: AtomicU64,
    cancellations: AtomicU64,
    bytes_sent: AtomicU64,
    bytes_received: AtomicU64,
    active_lanes: AtomicUsize,
}

struct ActiveRequest<'a> {
    metrics: &'a LocalMetrics,
}

impl LocalMetrics {
    fn begin(&self, bytes_sent: usize) -> ActiveRequest<'_> {
        self.requests.fetch_add(1, Ordering::Relaxed);
        self.bytes_sent
            .fetch_add(bytes_sent as u64, Ordering::Relaxed);
        self.active_lanes.fetch_add(1, Ordering::AcqRel);
        ActiveRequest { metrics: self }
    }

    fn record_get(&self, found: bool, bytes_received: usize) {
        if found {
            self.hits.fetch_add(1, Ordering::Relaxed);
            self.bytes_received
                .fetch_add(bytes_received as u64, Ordering::Relaxed);
        } else {
            self.misses.fetch_add(1, Ordering::Relaxed);
        }
    }
}

impl Drop for ActiveRequest<'_> {
    fn drop(&mut self) {
        self.metrics.active_lanes.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Closable Node-API handle shared by Node.js, Bun, and Deno.
#[napi]
pub struct NativeClient {
    client: RwLock<Option<Arc<ProtectedClient>>>,
    metrics: LocalMetrics,
    next_request_id: AtomicU64,
    requests: std::sync::Mutex<HashMap<u64, AbortHandle>>,
}

#[napi]
impl NativeClient {
    /// Returns a fresh request ID safe to represent as a JavaScript number.
    #[napi(js_name = "next_request_id")]
    pub fn next_request_id_for_js(&self) -> f64 {
        self.allocate_request_id() as f64
    }

    /// Aborts a queued or active native operation by request ID.
    #[napi]
    pub fn cancel(&self, request_id: f64) -> Result<bool> {
        let request_id = parse_request_id(request_id)?;
        let request = self
            .requests
            .lock()
            .map_err(|_| state_error("native request registry lock is poisoned"))?
            .remove(&request_id);
        if let Some(request) = request {
            request.abort();
            self.metrics.cancellations.fetch_add(1, Ordering::Relaxed);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Verifies that the server is reachable.
    #[napi]
    pub async fn ping(&self, request_id: Option<f64>) -> Result<()> {
        let _request = self.metrics.begin(0);
        let client = self.active_client()?;
        self.run_request(FfiOperation::Ping, request_id, None, async move {
            client
                .ping()
                .await
                .map(|_| ())
                .map_err(|error| native_core_error_for(error, FfiOperation::Ping))
        })
        .await
    }

    /// Retrieves exact decoded bytes or `null` when the key is absent.
    #[napi]
    pub async fn get(
        &self,
        key: Uint8Array,
        request_id: Option<f64>,
    ) -> Result<Option<Uint8Array>> {
        let client = self.active_client()?;
        let _request = self.metrics.begin(key.len());
        let key = key.to_vec();
        let outcome = self
            .run_request(FfiOperation::Get, request_id, None, async move {
                client
                    .get(&key)
                    .await
                    .map_err(|error| native_core_error_for(error, FfiOperation::Get))
            })
            .await?;
        let value = outcome.into_option().map(Uint8Array::new);
        self.metrics.record_get(
            value.is_some(),
            value.as_ref().map_or(0, |value| value.len()),
        );
        Ok(value)
    }

    /// Retrieves a core-owned canonical JSON value.
    ///
    /// Raw-formatted values are rejected instead of being silently coerced.
    #[napi(js_name = "get_json")]
    pub async fn get_json(
        &self,
        key: Uint8Array,
        request_id: Option<f64>,
    ) -> Result<Option<String>> {
        let client = self.active_client()?;
        let _request = self.metrics.begin(key.len());
        let key = key.to_vec();
        let outcome = self
            .run_request(FfiOperation::GetJson, request_id, None, async move {
                client
                    .get_value(&key)
                    .await
                    .map_err(|error| native_core_error_for(error, FfiOperation::GetJson))
            })
            .await?;
        match outcome {
            GetOutcome::NotFound => {
                self.metrics.record_get(false, 0);
                Ok(None)
            }
            GetOutcome::Found(Value::Json(value)) => {
                let serialized = canonical_json_bytes(&value)
                    .map_err(|error| native_core_error_for(error.into(), FfiOperation::GetJson))
                    .and_then(|bytes| String::from_utf8(bytes).map_err(native_error))?;
                self.metrics.record_get(true, serialized.len());
                Ok(Some(serialized))
            }
            GetOutcome::Found(Value::Raw(_)) => Err(native_core_error_for(
                openkache_client_core::value::Error::ExpectedRawValue.into(),
                FfiOperation::GetJson,
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
        ttl_ms: Option<f64>,
        mutation_id: Option<Uint8Array>,
        request_id: Option<f64>,
    ) -> Result<String> {
        let _request = self.metrics.begin(key.len() + value.len());
        self.store(
            request_id,
            key.as_ref(),
            value.as_ref().to_vec(),
            condition,
            ttl_ms,
            mutation_id.as_ref().map(Uint8Array::as_ref),
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
        ttl_ms: Option<f64>,
        mutation_id: Option<Uint8Array>,
        request_id: Option<f64>,
    ) -> Result<String> {
        let value = parse_json_value(value)?;
        let serialized_value = canonical_json_bytes(&value)
            .map_err(|error| native_core_error_for(error.into(), FfiOperation::SetJson))?;
        let options = parse_set_options(
            condition.as_deref(),
            ttl_ms,
            mutation_id.as_ref().map(Uint8Array::as_ref),
        )?;
        let client = self.active_client()?;
        let _request = self.metrics.begin(key.len() + serialized_value.len());
        let key = key.to_vec();
        self.run_request(
            FfiOperation::SetJson,
            request_id,
            options.mutation_id(),
            async move {
                client
                    .set_value(&key, Value::Json(value), options)
                    .await
                    .map(map_set_outcome)
                    .map_err(|error| native_core_error_for(error, FfiOperation::SetJson))
            },
        )
        .await
    }

    /// Deletes a key and reports whether it existed.
    #[napi]
    pub async fn delete(
        &self,
        key: Uint8Array,
        mutation_id: Option<Uint8Array>,
        request_id: Option<f64>,
    ) -> Result<bool> {
        let mutation_id = parse_mutation_id(mutation_id.as_ref().map(Uint8Array::as_ref))?
            .unwrap_or(random_mutation_id().map_err(native_core_error)?);
        let client = self.active_client()?;
        let _request = self.metrics.begin(key.len());
        let key = key.to_vec();
        let outcome = self
            .run_request(
                FfiOperation::Delete,
                request_id,
                Some(mutation_id),
                async move {
                    client
                        .delete_with_mutation_id(&key, mutation_id)
                        .await
                        .map_err(|error| native_core_error_for(error, FfiOperation::Delete))
                },
            )
            .await?;
        Ok(outcome == DeleteOutcome::Deleted)
    }

    /// Returns point-in-time native counters.
    #[napi(js_name = "metrics_snapshot")]
    pub fn metrics_snapshot(&self) -> Result<NativeMetricsSnapshot> {
        let snapshot = self.active_client()?.metrics_snapshot();
        Ok(NativeMetricsSnapshot {
            requests: self.metrics.requests.load(Ordering::Relaxed) as f64,
            hits: self.metrics.hits.load(Ordering::Relaxed) as f64,
            misses: self.metrics.misses.load(Ordering::Relaxed) as f64,
            retries: snapshot.retries as f64,
            reconnects: snapshot.reconnects as f64,
            cancellations: self.metrics.cancellations.load(Ordering::Relaxed) as f64,
            transport_errors: snapshot.transport_errors as f64,
            protocol_errors: snapshot.protocol_errors as f64,
            bytes_sent: self.metrics.bytes_sent.load(Ordering::Relaxed) as f64,
            bytes_received: self.metrics.bytes_received.load(Ordering::Relaxed) as f64,
            active_lanes: self.metrics.active_lanes.load(Ordering::Acquire) as f64,
        })
    }

    /// Returns the server's JSON statistics payload.
    #[napi]
    pub async fn stats(&self, request_id: Option<f64>) -> Result<String> {
        let _request = self.metrics.begin(0);
        let client = self.active_client()?;
        self.run_request(FfiOperation::Stats, request_id, None, async move {
            client
                .stats()
                .await
                .map_err(|error| native_core_error_for(error, FfiOperation::Stats))
        })
        .await
    }

    /// Requests a server durability barrier.
    #[napi]
    pub async fn sync(&self, request_id: Option<f64>) -> Result<()> {
        let _request = self.metrics.begin(0);
        let client = self.active_client()?;
        self.run_request(FfiOperation::Sync, request_id, None, async move {
            client
                .sync()
                .await
                .map_err(|error| native_core_error_for(error, FfiOperation::Sync))
        })
        .await
    }

    /// Closes the shared core client. Repeated calls are safe.
    #[napi]
    pub async fn close(&self) -> Result<()> {
        self.abort_all_requests();
        let client = self.take_client()?;
        if let Some(client) = client {
            client.close().await.map_err(native_core_error)?;
        }
        Ok(())
    }

    /// Drops the native handle without awaiting the core shutdown future.
    ///
    /// This is reserved for a JavaScript finalizer, where no promise can be observed.
    #[napi(js_name = "close_now")]
    pub fn close_now(&self) -> Result<()> {
        self.abort_all_requests();
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
            .unwrap_or_else(|| "closed".to_string()))
    }

    /// Reconnects the shared core without replaying a request.
    #[napi]
    pub async fn reconnect(&self, request_id: Option<f64>) -> Result<()> {
        let _request = self.metrics.begin(0);
        let client = self.active_client()?;
        self.run_request(FfiOperation::Reconnect, request_id, None, async move {
            client
                .reconnect()
                .await
                .map_err(|error| native_core_error_for(error, FfiOperation::Reconnect))
        })
        .await
    }

    /// Retrieves exact bytes for a fixed-size protocol item ID.
    #[napi(js_name = "raw_get")]
    pub async fn raw_get(
        &self,
        item_id: Uint8Array,
        request_id: Option<f64>,
    ) -> Result<Option<Uint8Array>> {
        let item_id = parse_item_id(item_id.as_ref())?;
        let client = self.active_client()?;
        let _request = self.metrics.begin(item_id.as_bytes().len());
        let outcome = self
            .run_request(FfiOperation::Get, request_id, None, async move {
                client
                    .raw()
                    .get(item_id)
                    .await
                    .map_err(|error| native_core_error_for(error, FfiOperation::Get))
            })
            .await?;
        let value = outcome
            .into_option()
            .map(|value| Uint8Array::new(value.into_bytes()));
        self.metrics.record_get(
            value.is_some(),
            value.as_ref().map_or(0, |value| value.len()),
        );
        Ok(value)
    }

    /// Stores exact bytes for a fixed-size protocol item ID.
    #[napi(js_name = "raw_set")]
    pub async fn raw_set(
        &self,
        item_id: Uint8Array,
        value: Uint8Array,
        condition: Option<String>,
        ttl_ms: Option<f64>,
        mutation_id: Option<Uint8Array>,
        request_id: Option<f64>,
    ) -> Result<String> {
        let item_id = parse_item_id(item_id.as_ref())?;
        let options = parse_set_options(
            condition.as_deref(),
            ttl_ms,
            mutation_id.as_ref().map(Uint8Array::as_ref),
        )?;
        let client = self.active_client()?;
        let _request = self.metrics.begin(item_id.as_bytes().len() + value.len());
        let value = value.to_vec();
        self.run_request(
            FfiOperation::Set,
            request_id,
            options.mutation_id(),
            async move {
                client
                    .raw()
                    .set(item_id, ItemValue::new(value), options)
                    .await
                    .map(map_set_outcome)
                    .map_err(|error| native_core_error_for(error, FfiOperation::Set))
            },
        )
        .await
    }

    /// Deletes a fixed-size protocol item ID.
    #[napi(js_name = "raw_delete")]
    pub async fn raw_delete(
        &self,
        item_id: Uint8Array,
        mutation_id: Option<Uint8Array>,
        request_id: Option<f64>,
    ) -> Result<bool> {
        let item_id = parse_item_id(item_id.as_ref())?;
        let mutation_id = parse_mutation_id(mutation_id.as_ref().map(Uint8Array::as_ref))?
            .unwrap_or(random_mutation_id().map_err(native_core_error)?);
        let client = self.active_client()?;
        let _request = self.metrics.begin(item_id.as_bytes().len());
        let outcome = self
            .run_request(
                FfiOperation::Delete,
                request_id,
                Some(mutation_id),
                async move {
                    client
                        .raw()
                        .delete_with_mutation_id(item_id, mutation_id)
                        .await
                        .map_err(|error| native_core_error_for(error, FfiOperation::Delete))
                },
            )
            .await?;
        Ok(outcome == DeleteOutcome::Deleted)
    }
}

impl NativeClient {
    async fn store(
        &self,
        request_id: Option<f64>,
        key: &[u8],
        value: Vec<u8>,
        condition: Option<String>,
        ttl_ms: Option<f64>,
        mutation_id: Option<&[u8]>,
    ) -> Result<String> {
        let options = parse_set_options(condition.as_deref(), ttl_ms, mutation_id)?;
        let client = self.active_client()?;
        let key = key.to_vec();
        self.run_request(
            FfiOperation::Set,
            request_id,
            options.mutation_id(),
            async move {
                client
                    .set(&key, value, options)
                    .await
                    .map(map_set_outcome)
                    .map_err(|error| native_core_error_for(error, FfiOperation::Set))
            },
        )
        .await
    }

    async fn run_request<T, F>(
        &self,
        operation: FfiOperation,
        request_id: Option<f64>,
        mutation_id: Option<MutationId>,
        future: F,
    ) -> Result<T>
    where
        F: Future<Output = Result<T>>,
    {
        let request_id = request_id
            .map(parse_request_id)
            .transpose()?
            .unwrap_or_else(|| self.allocate_request_id());
        let (abort, registration) = AbortHandle::new_pair();
        {
            let mut requests = self
                .requests
                .lock()
                .map_err(|_| state_error("native request registry lock is poisoned"))?;
            match requests.entry(request_id) {
                Entry::Vacant(entry) => {
                    entry.insert(abort);
                }
                Entry::Occupied(_) => {
                    return Err(invalid_argument(format!(
                        "request ID {request_id} is already active"
                    )));
                }
            }
        }
        let result = Abortable::new(future, registration).await;
        if let Ok(mut requests) = self.requests.lock() {
            requests.remove(&request_id);
        }
        match result {
            Ok(result) => result,
            Err(_) => Err(native_cancelled_error(operation, mutation_id)),
        }
    }

    fn allocate_request_id(&self) -> u64 {
        let request_id = self
            .next_request_id
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                let current = (1..=MAX_SAFE_REQUEST_ID)
                    .contains(&current)
                    .then_some(current)
                    .unwrap_or(1);
                Some(if current == MAX_SAFE_REQUEST_ID {
                    1
                } else {
                    current + 1
                })
            })
            .unwrap_or(1);
        if (1..=MAX_SAFE_REQUEST_ID).contains(&request_id) {
            request_id
        } else {
            1
        }
    }

    fn abort_all_requests(&self) {
        if let Ok(mut requests) = self.requests.lock() {
            for (_, request) in requests.drain() {
                request.abort();
            }
        }
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
    let mut trusted_certificates = Certificate::from_der_or_pem_chain(options.certificate.as_ref())
        .map_err(native_core_error)?;
    if trusted_certificates.len() != 1 {
        return Err(invalid_argument(format!(
            "certificate must contain exactly one DER or PEM certificate, got {}",
            trusted_certificates.len()
        )));
    }

    let data_protection_key = DataProtectionKey::from_slice(options.data_protection_key.as_ref())
        .map_err(native_core_error)?;
    let previous_keys = options.previous_data_protection_keys.unwrap_or_default();
    if previous_keys.len() > MAX_PREVIOUS_DATA_PROTECTION_KEYS {
        return Err(invalid_argument(format!(
            "previous_data_protection_keys may contain at most {MAX_PREVIOUS_DATA_PROTECTION_KEYS} entries"
        )));
    }
    let previous_keys = previous_keys
        .into_iter()
        .map(|key| DataProtectionKey::from_slice(key.as_ref()).map_err(native_core_error))
        .collect::<Result<Vec<_>>>()?;
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

    let retry = RetryPolicy::with_max_attempts(
        options
            .retry_max_attempts
            .map(|value| parse_usize(value, "retry_max_attempts", false))
            .transpose()?
            .unwrap_or(RetryPolicy::default().max_attempts),
    );
    let max_in_flight = options
        .max_in_flight
        .map(|value| parse_usize(value, "max_in_flight", false))
        .transpose()?
        .unwrap_or(DEFAULT_MAX_IN_FLIGHT);
    let encryption = parse_encryption(options.encryption.as_deref())?;
    let endpoint = parse_endpoint(&options.address, &options.server_name)?;
    let trusted_certificate = trusted_certificates.remove(0);
    let key_ring = DataProtectionKeyRing::with_previous(data_protection_key, previous_keys)
        .map_err(native_core_error)?;
    let mut builder = ProtectedClient::builder_with_key_ring(endpoint, key_ring)
        .trust_certificate(trusted_certificate)
        .compression(compression)
        .timeouts(timeouts)
        .retry_policy(retry)
        .max_in_flight(max_in_flight)
        .encryption(encryption);
    if let Some(identity) = identity {
        builder = builder.client_identity(identity);
    }
    let client = builder.connect().await.map_err(native_core_error)?;
    Ok(NativeClient {
        client: RwLock::new(Some(Arc::new(client))),
        metrics: LocalMetrics::default(),
        next_request_id: AtomicU64::new(1),
        requests: std::sync::Mutex::new(HashMap::new()),
    })
}

fn parse_identity(identity: Option<NativeIdentity>) -> Result<Option<ClientIdentity>> {
    let Some(identity) = identity else {
        return Ok(None);
    };
    let mut certificate_chain = Vec::new();
    for certificate in identity.certificate_chain {
        certificate_chain.extend(
            Certificate::from_der_or_pem_chain(certificate.as_ref()).map_err(native_core_error)?,
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
        .map_err(native_core_error)
}

fn parse_private_key(bytes: &[u8]) -> Result<PrivateKey> {
    PrivateKey::from_der_or_pem(bytes).map_err(native_core_error)
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

fn parse_set_options(
    condition: Option<&str>,
    ttl_ms: Option<f64>,
    mutation_id: Option<&[u8]>,
) -> Result<SetOptions> {
    let condition = parse_condition(condition)?;
    let ttl_ms = ttl_ms
        .map(|value| parse_u64(value, "ttl_ms", false))
        .transpose()?;
    let mut options = match condition {
        SetCondition::None => SetOptions::new(),
        SetCondition::IfAbsent => SetOptions::new().if_absent(),
        SetCondition::IfPresent => SetOptions::new().if_present(),
    };
    if let Some(ttl_ms) = ttl_ms {
        options = options.expires_after_millis(ttl_ms);
    }
    let mutation_id =
        parse_mutation_id(mutation_id)?.unwrap_or(random_mutation_id().map_err(native_core_error)?);
    options = options.with_mutation_id(mutation_id);
    Ok(options)
}

fn parse_mutation_id(bytes: Option<&[u8]>) -> Result<Option<MutationId>> {
    let Some(bytes) = bytes else {
        return Ok(None);
    };
    if bytes.len() != openkache_client_core::contract::MUTATION_ID_BYTES {
        return Err(invalid_argument(format!(
            "mutation_id must contain exactly {} bytes",
            openkache_client_core::contract::MUTATION_ID_BYTES
        )));
    }
    let bytes: [u8; openkache_client_core::contract::MUTATION_ID_BYTES] = bytes
        .try_into()
        .map_err(|_| invalid_argument("invalid mutation_id"))?;
    Ok(Some(MutationId::new(bytes)))
}

fn parse_item_id(bytes: &[u8]) -> Result<ItemId> {
    ItemId::from_slice(bytes).map_err(native_core_error)
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
    Endpoint::from_socket_addr(address, server_name).map_err(native_core_error)
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

fn parse_request_id(value: f64) -> Result<u64> {
    if !value.is_finite() || value.fract() != 0.0 || !(1.0..=MAX_SAFE_INTEGER).contains(&value) {
        return Err(invalid_argument(
            "request_id must be a positive safe integer",
        ));
    }
    Ok(value as u64)
}

fn map_set_outcome(outcome: SetOutcome) -> String {
    match outcome {
        SetOutcome::Created => "created".to_string(),
        SetOutcome::Replaced => "replaced".to_string(),
        SetOutcome::NotStored => "not_stored".to_string(),
    }
}

fn native_error(error: impl std::fmt::Display) -> Error {
    Error::new(Status::GenericFailure, error.to_string())
}

#[derive(Clone, Copy, Debug, Default)]
struct NativeErrorMetadata {
    code: u32,
    operation: u32,
    phase: u32,
    backend: u32,
    retryable: bool,
    ambiguous: bool,
    mutation_id: Option<[u8; openkache_client_core::contract::MUTATION_ID_BYTES]>,
}

fn native_cancelled_error(operation: FfiOperation, mutation_id: Option<MutationId>) -> Error {
    let has_mutation = mutation_id.is_some();
    native_error_with_metadata(
        "client operation canceled",
        NativeErrorMetadata {
            code: FFI_ERROR_CANCELLED,
            operation: operation.code(),
            retryable: has_mutation,
            // This adapter aborts the native future directly and cannot
            // observe whether a partial write reached the transport. A
            // mutation token makes the conservative outcome reusable: the
            // caller can retry it safely through the server dedupe store.
            ambiguous: has_mutation,
            mutation_id: mutation_id.map(MutationId::into_bytes),
            ..NativeErrorMetadata::default()
        },
    )
}

fn native_error_with_metadata(message: impl Into<String>, metadata: NativeErrorMetadata) -> Error {
    let reason = serde_json::json!({
        "__openkache_native_error": true,
        "message": message.into(),
        "metadata": {
            "code": metadata.code,
            "operation": metadata.operation,
            "phase": metadata.phase,
            "backend": metadata.backend,
            "retryable": metadata.retryable,
            "ambiguous": metadata.ambiguous,
            "mutation_id": metadata.mutation_id.map(|value| value.to_vec()),
        },
    });
    Error::new(Status::GenericFailure, reason.to_string())
}

/// Encodes the core's structured error metadata into the message of a N-API error.
///
/// N-API's `Result<T>` conversion only exposes the status and reason fields of
/// `napi::Error`.  A small, private JSON envelope keeps the public JavaScript
/// error message stable while allowing the TypeScript adapter to recover the
/// ABI-v3 metadata without a second native call.
fn native_core_error(error: CoreError) -> Error {
    let metadata = core_error_metadata(&error, None);
    native_error_with_metadata(error.to_string(), metadata)
}

fn native_core_error_for(error: CoreError, operation: FfiOperation) -> Error {
    let metadata = core_error_metadata(&error, Some(operation));
    native_error_with_metadata(error.to_string(), metadata)
}

fn core_error_metadata(
    error: &CoreError,
    caller_operation: Option<FfiOperation>,
) -> NativeErrorMetadata {
    let mut metadata = NativeErrorMetadata {
        operation: caller_operation.map_or(0, FfiOperation::code),
        ..NativeErrorMetadata::default()
    };
    match error {
        CoreError::Configuration { .. } => metadata.code = FFI_ERROR_CONFIGURATION,
        CoreError::Connection(_) => {
            metadata.code = FFI_ERROR_CONNECTION;
            metadata.retryable = true;
        }
        CoreError::Timeout { operation } => {
            metadata.code = FFI_ERROR_TIMEOUT;
            if caller_operation.is_none() {
                metadata.operation = operation.ffi_operation_code();
            }
            metadata.phase = operation.ffi_phase_code();
            metadata.retryable = true;
        }
        CoreError::Runtime { backend, .. } => {
            metadata.code = FFI_ERROR_RUNTIME;
            metadata.backend = backend.ffi_code();
        }
        CoreError::Transport {
            backend, operation, ..
        } => {
            metadata.code = FFI_ERROR_TRANSPORT;
            metadata.backend = backend.ffi_code();
            if caller_operation.is_none() {
                metadata.operation = operation.ffi_operation_code();
            }
            metadata.phase = operation.ffi_phase_code();
            metadata.retryable = true;
        }
        CoreError::Server { code, .. } => {
            metadata.code = FFI_ERROR_SERVER;
            metadata.retryable =
                matches!(code, ServerErrorCode::Overloaded | ServerErrorCode::Timeout);
        }
        CoreError::UnexpectedResponse { operation, .. } => {
            metadata.code = FFI_ERROR_UNEXPECTED_RESPONSE;
            if caller_operation.is_none() {
                metadata.operation = operation.ffi_operation_code();
            }
            metadata.phase = operation.ffi_phase_code();
        }
        CoreError::ResponseTooLarge { .. } => metadata.code = FFI_ERROR_RESPONSE_TOO_LARGE,
        CoreError::Tls(_) => metadata.code = FFI_ERROR_TLS,
        CoreError::Protocol(_) => metadata.code = FFI_ERROR_PROTOCOL,
        CoreError::Io(_) => {
            metadata.code = FFI_ERROR_IO;
            metadata.retryable = true;
        }
        CoreError::Value(_) => metadata.code = FFI_ERROR_VALUE,
        CoreError::ClientClosed => metadata.code = FFI_ERROR_CLOSED,
        CoreError::AmbiguousOutcome {
            operation,
            mutation_id,
            cause,
        } => {
            metadata.code = FFI_ERROR_AMBIGUOUS;
            if caller_operation.is_none() {
                metadata.operation = operation.ffi_operation_code();
            }
            metadata.ambiguous = true;
            metadata.retryable = true;
            metadata.mutation_id = mutation_id.map(MutationId::into_bytes);
            let nested = core_error_metadata(cause, caller_operation);
            metadata.phase = nested.phase;
            metadata.backend = nested.backend;
        }
        _ => {}
    }
    metadata
}

fn invalid_argument(message: impl Into<String>) -> Error {
    Error::new(Status::InvalidArg, message.into())
}

fn state_error(message: impl Into<String>) -> Error {
    Error::new(Status::GenericFailure, message.into())
}
