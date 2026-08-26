#![doc = include_str!("../README.md")]
#![doc(html_root_url = "https://docs.rs/openkache/0.1.1")]

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

mod native;

mod maintained {
    use std::fmt;

    use super::internal_core as core;
    use super::native::{ValueCodec as ValueCodecTrait, from_value, to_value};
    use core::value::{Compression, Encryption};
    use core::{GetOutcome as CoreGetOutcome, Operation, SetOutcome as CoreSetOutcome};
    use serde::Serialize;
    use serde::de::DeserializeOwned;

    pub use super::internal_value::{
        Error as ValueError, Float, FloatWidth, Integer, Limits as ValueLimits, Sign, Value,
    };
    pub use super::native::{FunctionCodec, SerdeCodec, ValueCodec};
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

    /// A mutation whose response was lost after admission.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum Mutation {
        /// The unknown operation was `set`.
        Set,
        /// The unknown operation was `delete`.
        Delete,
    }

    /// Errors returned by the client.
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
        /// The server returned a conditional-set status even though this
        /// client uses unconditional writes.
        #[error("server returned an unsupported set outcome")]
        UnsupportedSetOutcome,
        /// Native Serde serialization failed before the request was admitted.
        #[error("native serialization failed: {0}")]
        NativeSerialize(String),
        /// Native Serde deserialization failed after the value was retrieved.
        #[error("native deserialization failed: {0}")]
        NativeDeserialize(String),
        /// A custom [`ValueCodec`] failed before a write was admitted.
        #[error("value codec encoding failed: {0}")]
        CodecEncode(String),
        /// A custom [`ValueCodec`] failed after a value was retrieved.
        #[error("value codec decoding failed: {0}")]
        CodecDecode(String),
    }

    /// Result alias for client operations.
    pub type Result<T> = std::result::Result<T, Error>;

    impl<T> GetResult<T> {
        /// Returns the found value as `Some`, or `None` for a missing key.
        pub fn into_option(self) -> Option<T> {
            match self {
                Self::Missing => None,
                Self::Found(value) => Some(value),
            }
        }

        /// Returns whether the lookup found an item.
        pub fn is_found(&self) -> bool {
            matches!(self, Self::Found(_))
        }

        /// Returns whether the lookup did not find an item.
        pub fn is_missing(&self) -> bool {
            matches!(self, Self::Missing)
        }

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

    #[cfg(feature = "quic-compio")]
    use core::LocalProtectedClient;
    #[cfg(feature = "quic-quinn")]
    use core::ProtectedClient;

    /// A connected OpenKache client.
    ///
    /// Values use the OpenKache structured value format. Connections use the
    /// local development TLS profile described in [`Client::connect`].
    #[cfg(feature = "quic-quinn")]
    #[derive(Clone)]
    pub struct Client {
        inner: ProtectedClient,
    }

    #[cfg(feature = "quic-quinn")]
    impl Client {
        async fn gate0_client(&self) -> Result<&ProtectedClient> {
            let namespace_id = self
                .inner
                .raw()
                .ensure_namespace_id()
                .await
                .map_err(map_core_error)?;
            if namespace_id != core::contract::GATE0_NAMESPACE_ID {
                return Err(Error::Core(format!(
                    "server selected namespace {namespace_id}, expected Gate 0 namespace {}",
                    core::contract::GATE0_NAMESPACE_ID,
                )));
            }
            Ok(&self.inner)
        }

        /// Connects to an OpenKache server using the local development profile.
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
            .compression(Compression::Disabled)
            .encryption(Encryption::Unprotected)
            .connect()
            .await
            .map_err(map_core_error)?;
            Ok(Self { inner })
        }

        /// Retrieves one structured value.
        ///
        /// # Arguments
        ///
        /// * `key` - A text, byte, or signed integer key convertible to
        ///   [`TypedKey`].
        ///
        /// # Returns
        ///
        /// `Ok(GetResult::Found(value))` when the item exists, or
        /// `Ok(GetResult::Missing)` when it does not.
        ///
        /// # Errors
        ///
        /// Returns [`Error`] when the connection, protocol, key, or value
        /// operation fails.
        pub async fn get(&self, key: impl Into<TypedKey>) -> Result<GetResult<Value>> {
            self.gate0_client()
                .await?
                .get_structured(key)
                .await
                .map(|outcome| match outcome {
                    CoreGetOutcome::Found(value) => GetResult::Found(value),
                    CoreGetOutcome::NotFound => GetResult::Missing,
                })
                .map_err(map_core_error)
        }

        /// Stores one structured value using an unconditional write.
        ///
        /// Native Rust strings, byte vectors, booleans, integers, and floats
        /// convert directly to [`Value`]. Use an explicit [`Value`] variant
        /// when the exact model representation matters.
        ///
        /// # Arguments
        ///
        /// * `key` - A text, byte, or signed integer key convertible to
        ///   [`TypedKey`].
        /// * `value` - A native value or [`Value`].
        ///
        /// # Returns
        ///
        /// [`SetOutcome::Created`] for a new item or
        /// [`SetOutcome::Replaced`] when an existing item is overwritten.
        ///
        /// # Errors
        ///
        /// Returns [`Error`] when the connection, protocol, value, or storage
        /// operation fails.
        pub async fn set(
            &self,
            key: impl Into<TypedKey>,
            value: impl Into<Value>,
        ) -> Result<SetOutcome> {
            let value = value.into();
            self.gate0_client()
                .await?
                .set_structured(key, value, core::SetOptions::new())
                .await
                .map_err(map_core_error)
                .and_then(|outcome| match outcome {
                    CoreSetOutcome::Created => Ok(SetOutcome::Created),
                    CoreSetOutcome::Replaced => Ok(SetOutcome::Replaced),
                    CoreSetOutcome::NotStored => Err(Error::UnsupportedSetOutcome),
                })
        }

        /// Deletes one key and reports whether an item was removed.
        pub async fn delete(&self, key: impl Into<TypedKey>) -> Result<bool> {
            self.gate0_client()
                .await?
                .delete(key)
                .await
                .map_err(map_core_error)
                .map(|outcome| match outcome {
                    core::DeleteOutcome::Deleted => true,
                    core::DeleteOutcome::NotFound => false,
                })
        }

        /// Idempotently closes the client and waits for admitted work to
        /// settle before releasing the transport.
        pub async fn close(&self) -> Result<()> {
            self.inner.close().await.map_err(map_core_error)
        }
    }

    /// A connected Gate 0 client running on Compio.
    ///
    /// `CompioClient` has the same operation and value contract as [`Client`],
    /// but it does not create or require a Tokio runtime. Call it from an
    /// active Compio runtime, for example one created with
    /// `compio::runtime::Runtime::new()`.
    #[cfg(feature = "quic-compio")]
    #[derive(Clone)]
    pub struct CompioClient {
        inner: LocalProtectedClient,
    }

    #[cfg(feature = "quic-compio")]
    impl CompioClient {
        async fn gate0_client(&self) -> Result<&LocalProtectedClient> {
            let namespace_id = self
                .inner
                .raw()
                .ensure_namespace_id()
                .await
                .map_err(map_core_error)?;
            if namespace_id != core::contract::GATE0_NAMESPACE_ID {
                return Err(Error::Core(format!(
                    "server selected namespace {namespace_id}, expected Gate 0 namespace {}",
                    core::contract::GATE0_NAMESPACE_ID,
                )));
            }
            Ok(&self.inner)
        }

        /// Connects using the fixed Gate 0 development profile on Compio.
        ///
        /// The profile intentionally disables certificate verification for
        /// local development. It still requires a TLS 1.3 handshake and never
        /// falls back to plaintext. Do not use this trust profile in production.
        pub async fn connect(endpoint: impl AsRef<str>) -> Result<Self> {
            let endpoint = endpoint.as_ref().parse().map_err(map_core_error)?;
            let inner = LocalProtectedClient::builder(
                endpoint,
                core::DataProtectionKey::from_bytes(core::contract::GATE0_ITEM_ID_ROOT),
            )
            .server_trust(core::ServerTrust::Insecure)
            .compression(Compression::Disabled)
            .encryption(Encryption::Unprotected)
            .connect()
            .await
            .map_err(map_core_error)?;
            Ok(Self { inner })
        }

        /// Retrieves one lossless structured value.
        pub async fn get(&self, key: impl Into<TypedKey>) -> Result<GetResult<Value>> {
            self.gate0_client()
                .await?
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
            self.gate0_client()
                .await?
                .set_structured(key, value, core::SetOptions::new())
                .await
                .map_err(map_core_error)
                .and_then(|outcome| match outcome {
                    CoreSetOutcome::Created => Ok(SetOutcome::Created),
                    CoreSetOutcome::Replaced => Ok(SetOutcome::Replaced),
                    CoreSetOutcome::NotStored => Err(Error::UnsupportedSetOutcome),
                })
        }

        /// Retrieves one value and decodes it into a native Serde type.
        ///
        /// Serialization is performed against the same lossless structured
        /// value model as [`Client::set`]. A missing key remains
        /// [`GetResult::Missing`], while a stored null decodes as `None` for
        /// an `Option<T>`.
        ///
        /// # Errors
        ///
        /// Returns [`Error::NativeDeserialize`] when the stored value cannot
        /// be represented by `T`, and [`Error::Core`] for transport,
        /// protocol, key, or value failures.
        pub async fn get_native<T: DeserializeOwned>(
            &self,
            key: impl Into<TypedKey>,
        ) -> Result<GetResult<T>> {
            match self.get(key).await? {
                GetResult::Missing => Ok(GetResult::Missing),
                GetResult::Found(value) => from_value(value)
                    .map(GetResult::Found)
                    .map_err(|error| Error::NativeDeserialize(error.to_string())),
            }
        }

        /// Serializes a native Serde value and stores it with an unconditional
        /// write.
        ///
        /// Native serialization completes before the request is admitted, so
        /// a serialization error cannot produce [`Error::UnknownMutation`].
        ///
        /// # Errors
        ///
        /// Returns [`Error::NativeSerialize`] when `value` cannot be represented
        /// by the structured value model. Transport and protocol failures use
        /// the same errors as [`Client::set`].
        pub async fn set_native<T: Serialize>(
            &self,
            key: impl Into<TypedKey>,
            value: T,
        ) -> Result<SetOutcome> {
            let value =
                to_value(&value).map_err(|error| Error::NativeSerialize(error.to_string()))?;
            self.set(key, value).await
        }

        /// Retrieves one value and decodes it with an application codec.
        ///
        /// # Errors
        ///
        /// Returns [`Error::CodecDecode`] when `codec` rejects the stored
        /// value, or the same transport and protocol errors as [`Client::get`].
        pub async fn get_with<T, C>(
            &self,
            key: impl Into<TypedKey>,
            codec: &C,
        ) -> Result<GetResult<T>>
        where
            C: ValueCodecTrait<T>,
        {
            match self.get(key).await? {
                GetResult::Missing => Ok(GetResult::Missing),
                GetResult::Found(value) => codec
                    .decode(value)
                    .map(GetResult::Found)
                    .map_err(|error| Error::CodecDecode(error.to_string())),
            }
        }

        /// Encodes a native value with an application codec and stores it with
        /// an unconditional write.
        ///
        /// Encoding completes before request admission, so a codec failure
        /// cannot produce [`Error::UnknownMutation`].
        ///
        /// # Errors
        ///
        /// Returns [`Error::CodecEncode`] when `codec` rejects `value`, or the
        /// same transport and mutation errors as [`Client::set`].
        pub async fn set_with<T, C>(
            &self,
            key: impl Into<TypedKey>,
            value: &T,
            codec: &C,
        ) -> Result<SetOutcome>
        where
            C: ValueCodecTrait<T>,
        {
            let value = codec
                .encode(value)
                .map_err(|error| Error::CodecEncode(error.to_string()))?;
            self.set(key, value).await
        }

        /// Deletes one key and reports whether an item was removed.
        pub async fn delete(&self, key: impl Into<TypedKey>) -> Result<bool> {
            self.gate0_client()
                .await?
                .delete(key)
                .await
                .map_err(map_core_error)
                .map(|outcome| match outcome {
                    core::DeleteOutcome::Deleted => true,
                    core::DeleteOutcome::NotFound => false,
                })
        }

        /// Idempotently closes the client and waits for admitted work to
        /// settle before releasing the transport.
        pub async fn close(&self) -> Result<()> {
            self.inner.close().await.map_err(map_core_error)
        }
    }

    /// Compatibility name for callers that use the core's local-runtime
    /// terminology. Prefer [`CompioClient`] in new code.
    #[cfg(feature = "quic-compio")]
    pub type LocalClient = CompioClient;
}

pub use maintained::*;
