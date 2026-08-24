//! The maintained OpenKache Rust client.
//!
//! The published facade intentionally has one small surface: [`Client::connect`],
//! [`Client::get`], [`Client::set`], [`Client::delete`], and [`Client::close`].
//! Gate 0 fixes the development transport profile, NamespaceHash key mapping,
//! and `StructuredValue-CBOR-v1`; callers cannot select certificates,
//! protection, compression, retries, or cancellation.

#![doc(html_root_url = "https://docs.rs/openkache/0.1.0")]

#[path = "internal/core/lib.rs"]
#[allow(
    dead_code,
    rustdoc::private_intra_doc_links,
    unexpected_cfgs,
    unused_imports
)]
mod internal_core;
#[path = "internal/protocol/lib.rs"]
#[allow(dead_code, unexpected_cfgs, unused_imports)]
mod internal_protocol;
#[path = "internal/value/lib.rs"]
#[allow(dead_code, unexpected_cfgs, unused_imports)]
mod internal_value;

mod maintained {
    use std::fmt;

    use super::internal_core as core;
    use core::value::{Compression, Encryption};
    use core::{GetOutcome as CoreGetOutcome, Operation, SetOutcome as CoreSetOutcome};

    pub use super::internal_value::{
        Error as ValueError, Float, FloatWidth, Integer, Limits as ValueLimits, Sign, Value,
    };
    pub use core::KeyError;
    pub use core::TypedKey;

    /// A successful lookup result. `Missing` is distinct from every stored
    /// value, including `Value::Null` and `Value::Undefined`.
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub enum GetResult<T> {
        /// No item exists for the supplied key.
        Missing,
        /// The item exists and contains the supplied value.
        Found(T),
    }

    /// Result of an unconditional `set`.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum SetOutcome {
        /// The key was absent and a new item was created.
        Created,
        /// An existing item was replaced.
        Replaced,
    }

    /// Result of an idempotent `delete`.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum DeleteOutcome {
        /// An existing item was removed.
        Deleted,
        /// No item existed for the supplied key.
        NotFound,
    }

    /// A mutation whose response was lost after admission.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum Mutation {
        /// The unknown operation was `set`.
        Set,
        /// The unknown operation was `delete`.
        Delete,
    }

    /// Errors returned by the maintained facade.
    #[derive(Debug, thiserror::Error)]
    pub enum Error {
        /// A mutation may have taken effect, but the response could not be
        /// confirmed. The client never retries this operation automatically.
        #[error("{operation:?} result is unknown after request admission")]
        UnknownMutation {
            /// The mutation whose result is unknown.
            operation: Mutation,
        },
        /// A connection, protocol, key, or value failure from the internal
        /// implementation.
        #[error("client operation failed: {0}")]
        Core(String),
        /// The server returned a conditional-set status even though Gate 0
        /// never sends conditional options.
        #[error("server returned an unsupported set outcome")]
        UnsupportedSetOutcome,
    }

    /// Result alias for the maintained facade.
    pub type Result<T> = std::result::Result<T, Error>;

    impl<T> GetResult<T> {
        /// Applies a function only when the lookup found an item.
        pub fn map<U>(self, function: impl FnOnce(T) -> U) -> GetResult<U> {
            match self {
                Self::Missing => GetResult::Missing,
                Self::Found(value) => GetResult::Found(function(value)),
            }
        }
    }

    impl fmt::Display for Mutation {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str(match self {
                Self::Set => "set",
                Self::Delete => "delete",
            })
        }
    }

    fn map_core_error(error: core::Error) -> Error {
        match error {
            core::Error::AmbiguousOutcome { operation, .. } => {
                let operation = match operation {
                    Operation::Delete => Mutation::Delete,
                    _ => Mutation::Set,
                };
                Error::UnknownMutation { operation }
            }
            other => Error::Core(other.to_string()),
        }
    }

    #[cfg(feature = "quic-quinn")]
    use core::ProtectedClient;

    /// A connected Gate 0 client.
    ///
    /// The only supported operations are [`Client::get`], [`Client::set`],
    /// [`Client::delete`], and [`Client::close`]. Values are always
    /// `StructuredValue-CBOR-v1`; the fixed profile is uncompressed and
    /// unprotected inside TLS.
    #[cfg(feature = "quic-quinn")]
    #[derive(Clone)]
    pub struct Client {
        inner: ProtectedClient,
    }

    #[cfg(feature = "quic-quinn")]
    impl Client {
        /// Connects using the fixed Gate 0 development profile.
        ///
        /// The profile intentionally disables certificate verification for
        /// local development. It still requires a TLS 1.3 handshake and never
        /// falls back to plaintext. Do not use this trust profile in production.
        pub async fn connect(endpoint: impl AsRef<str>) -> Result<Self> {
            let endpoint = endpoint.as_ref().parse().map_err(map_core_error)?;
            let inner = ProtectedClient::builder(
                endpoint,
                core::DataProtectionKey::from_bytes(core::contract::GATE0_ITEM_ID_ROOT),
            )
            .server_trust(core::ServerTrust::Insecure)
            .namespace_id(core::contract::GATE0_NAMESPACE_ID)
            .compression(Compression::Disabled)
            .encryption(Encryption::Unprotected)
            .connect()
            .await
            .map_err(map_core_error)?;
            Ok(Self { inner })
        }

        /// Retrieves one lossless structured value.
        pub async fn get(&self, key: impl Into<TypedKey>) -> Result<GetResult<Value>> {
            self.inner
                .get_structured(key)
                .await
                .map(|outcome| match outcome {
                    CoreGetOutcome::Found(value) => GetResult::Found(value),
                    CoreGetOutcome::NotFound => GetResult::Missing,
                })
                .map_err(map_core_error)
        }

        /// Stores one lossless structured value using an unconditional write.
        pub async fn set(&self, key: impl Into<TypedKey>, value: Value) -> Result<SetOutcome> {
            self.inner
                .set_structured(key, value, core::SetOptions::new())
                .await
                .map_err(map_core_error)
                .and_then(|outcome| match outcome {
                    CoreSetOutcome::Created => Ok(SetOutcome::Created),
                    CoreSetOutcome::Replaced => Ok(SetOutcome::Replaced),
                    CoreSetOutcome::NotStored => Err(Error::UnsupportedSetOutcome),
                })
        }

        /// Deletes one key. Repeating the operation is safe and returns
        /// [`DeleteOutcome::NotFound`].
        pub async fn delete(&self, key: impl Into<TypedKey>) -> Result<DeleteOutcome> {
            self.inner
                .delete(key)
                .await
                .map_err(map_core_error)
                .map(|outcome| match outcome {
                    core::DeleteOutcome::Deleted => DeleteOutcome::Deleted,
                    core::DeleteOutcome::NotFound => DeleteOutcome::NotFound,
                })
        }

        /// Idempotently closes the client and waits for admitted work to
        /// settle before releasing the transport.
        pub async fn close(&self) -> Result<()> {
            self.inner.close().await.map_err(map_core_error)
        }
    }
}

pub use maintained::*;
