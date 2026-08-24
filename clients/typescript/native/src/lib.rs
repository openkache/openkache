//! Private Node-API adapter for the OpenKache v1 Gate 0 client.
//!
//! Gate 0 intentionally fixes the transport, trust, key, namespace, and value
//! profiles.  The adapter keeps those settings below the TypeScript boundary;
//! callers can provide only an endpoint and the five maintained operations.

use std::sync::{Arc, RwLock};

use napi::bindgen_prelude::Uint8Array;
use napi::{Error, Result, Status};
use napi_derive::napi;
use openkache_client_core::value::{Compression, Encryption};
use openkache_client_core::{
    ClientRootKey, DeleteOutcome, Endpoint, GetOutcome, ProtectedClient, ServerTrust, SetOptions,
    SetOutcome,
};

const ITEM_ID_ROOT: [u8; 32] = [
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
    0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f,
];

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
        let outcome = self
            .active_client()?
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
        let outcome = self
            .active_client()?
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
        let outcome = self
            .active_client()?
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
}

/// Connects using the fixed Gate 0 development profile.
///
/// The server certificate is presented and TLS 1.3 encrypts the connection,
/// but `DevelopmentTrust` deliberately disables certificate and hostname
/// verification.  This profile is development only — do not use it in
/// production.
#[napi]
pub async fn connect(options: NativeClientOptions) -> Result<NativeClient> {
    let address = options.address.parse().map_err(|error| {
        invalid_argument(format!(
            "invalid server address {:?}: {error}",
            options.address
        ))
    })?;
    let endpoint = Endpoint::from_socket_addr(address, "localhost").map_err(native_core_error)?;
    let client = ProtectedClient::builder(endpoint, ClientRootKey::from_bytes(ITEM_ID_ROOT))
        .server_trust(ServerTrust::Insecure)
        .compression(Compression::Disabled)
        .encryption(Encryption::Unprotected)
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
    Error::new(Status::GenericFailure, message.into())
}

fn state_error(message: impl Into<String>) -> Error {
    Error::new(Status::GenericFailure, message.into())
}
