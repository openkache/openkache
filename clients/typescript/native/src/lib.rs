//! Private Node-API adapter for the OpenKache v1 Gate 0 client.
//!
//! Gate 0 intentionally fixes the transport, trust, key, namespace, and value
//! profiles.  The adapter keeps those settings below the TypeScript boundary;
//! callers can provide only an endpoint and the five maintained operations.

mod gate0_contract;

use std::sync::{Arc, RwLock};

use napi::bindgen_prelude::Uint8Array;
use napi::{Error, Result, Status};
use napi_derive::napi;
use openkache_client_core::{
    DeleteOutcome, Endpoint, GetOutcome, ProtectedClient, SetOptions, SetOutcome,
};

/// Endpoint-only connection settings consumed by the private native loader.
#[napi(object)]
pub struct NativeClientOptions {
    pub address: String,
}

/// Closable native client handle shared by Node.js, Bun, and Deno.
#[napi]
pub struct NativeClient {
    client: RwLock<Option<Arc<ProtectedClient>>>,
}

#[napi]
impl NativeClient {
    /// Retrieves one canonical StructuredValue-CBOR-v1 payload.
    ///
    /// `None` is an internal FFI sentinel only; the TypeScript adapter maps it
    /// to its explicit `Missing_Result` value.
    #[napi]
    pub async fn get(&self, key: Uint8Array) -> Result<Option<Uint8Array>> {
        let client = self.gate0_client().await?;
        let outcome = client
            .get_structured_canonical_key_cbor(key.as_ref())
            .await
            .map_err(native_core_error)?;
        match outcome {
            GetOutcome::NotFound => Ok(None),
            GetOutcome::Found(payload) => Ok(Some(Uint8Array::new(payload))),
        }
    }

    /// Stores one canonical StructuredValue-CBOR-v1 payload unconditionally.
    #[napi]
    pub async fn set(&self, key: Uint8Array, value: Uint8Array) -> Result<String> {
        let client = self.gate0_client().await?;
        let outcome = client
            .set_structured_canonical_key_cbor(key.as_ref(), value.as_ref(), SetOptions::new())
            .await
            .map_err(native_core_error)?;
        match outcome {
            SetOutcome::Created => Ok("created".to_owned()),
            SetOutcome::Replaced => Ok("replaced".to_owned()),
            SetOutcome::NotStored => Err(incompatible_outcome(
                "server returned conditional SET outcome NotStored",
            )),
        }
    }

    /// Deletes one mapped key and reports whether it existed.
    #[napi]
    pub async fn delete(&self, key: Uint8Array) -> Result<bool> {
        let client = self.gate0_client().await?;
        let outcome = client
            .delete_canonical_key(key.as_ref())
            .await
            .map_err(native_core_error)?;
        Ok(outcome == DeleteOutcome::Deleted)
    }

    /// Closes the native connection. Repeated calls are safe.
    #[napi]
    pub async fn close(&self) -> Result<()> {
        let client = self.take_client()?;
        if let Some(client) = client {
            client.close().await.map_err(native_core_error)?;
        }
        Ok(())
    }

    /// Drops the native handle without awaiting shutdown.
    ///
    /// This is used only by the JavaScript finalizer, which cannot observe a
    /// promise or report an asynchronous error.
    #[napi(js_name = "close_now")]
    pub fn close_now(&self) -> Result<()> {
        self.take_client()?;
        Ok(())
    }
}

impl NativeClient {
    fn take_client(&self) -> Result<Option<Arc<ProtectedClient>>> {
        self.client
            .write()
            .map_err(|_| state_error("native client state lock is poisoned"))
            .map(|mut client| client.take())
    }

    fn active_client(&self) -> Result<Arc<ProtectedClient>> {
        self.client
            .read()
            .map_err(|_| state_error("native client state lock is poisoned"))?
            .as_ref()
            .map(Arc::clone)
            .ok_or_else(|| state_error("client is closed"))
    }

    async fn gate0_client(&self) -> Result<Arc<ProtectedClient>> {
        let client = self.active_client()?;
        let namespace_id = client
            .raw()
            .ensure_namespace_id()
            .await
            .map_err(native_core_error)?;
        if namespace_id != gate0_contract::namespace_id() {
            return Err(incompatible_outcome(format!(
                "server selected namespace {namespace_id}, expected Gate 0 namespace {}",
                gate0_contract::namespace_id(),
            )));
        }
        Ok(client)
    }
}

/// Connects using the fixed Gate 0 development profile.
///
/// The server certificate is presented and TLS 1.3 encrypts the connection,
/// but `DevelopmentTrust` deliberately disables certificate and hostname
/// verification.  This profile is development only — do not use it in
/// production.
#[napi]
pub async fn connect(options: NativeClientOptions) -> Result<NativeClient> {
    let profile = gate0_contract::profile();
    let address = options.address.parse().map_err(|error| {
        invalid_argument(format!(
            "invalid server address {:?}: {error}",
            options.address
        ))
    })?;
    let endpoint =
        Endpoint::from_socket_addr(address, profile.server_name).map_err(native_core_error)?;
    let client = ProtectedClient::builder(endpoint, profile.item_id_root)
        .server_trust(profile.server_trust)
        .alpn_policy(profile.alpn)
        .compression(profile.compression)
        .encryption(profile.encryption)
        .connect()
        .await
        .map_err(native_core_error)?;
    Ok(NativeClient {
        client: RwLock::new(Some(Arc::new(client))),
    })
}

fn native_core_error(error: openkache_client_core::Error) -> Error {
    let message = error.to_string();
    if matches!(error, openkache_client_core::Error::AmbiguousOutcome { .. }) {
        Error::new(
            Status::GenericFailure,
            format!("openkache:error:unknown_mutation:{message}"),
        )
    } else {
        Error::new(Status::GenericFailure, message)
    }
}

fn invalid_argument(message: impl Into<String>) -> Error {
    Error::new(Status::InvalidArg, message.into())
}

fn incompatible_outcome(message: impl Into<String>) -> Error {
    Error::new(
        Status::GenericFailure,
        format!(
            "openkache:error:incompatible_server_outcome:{}",
            message.into()
        ),
    )
}

fn state_error(message: impl Into<String>) -> Error {
    Error::new(Status::GenericFailure, message.into())
}
