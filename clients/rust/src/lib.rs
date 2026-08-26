#![doc = include_str!("../README.md")]
#![doc(html_root_url = "https://docs.rs/openkache/0.1.3")]

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
        ValueKind,
    };
    pub use super::native::{FunctionCodec, SerdeCodec, ValueCodec};
    pub use core::KeyError;
    pub use core::TypedKey;

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
        /// Serde serialization failed before the request was admitted.
        #[error("serde serialization failed: {0}")]
        SerdeSerialize(String),
        /// Serde deserialization failed after the value was retrieved.
        #[error("serde deserialization failed: {0}")]
        SerdeDeserialize(String),
        /// A custom [`ValueCodec`] failed before a write was admitted.
        #[error("value codec encoding failed: {0}")]
        CodecEncode(String),
        /// A custom [`ValueCodec`] failed after a value was retrieved.
        #[error("value codec decoding failed: {0}")]
        CodecDecode(String),
    }

    /// Stable high-level category for a client error.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum ErrorKind {
        /// A mutation may have taken effect, but its result is unknown.
        UnknownMutation,
        /// A connection, protocol, key, or value operation failed.
        Core,
        /// The server returned an unsupported set outcome.
        UnsupportedSetOutcome,
        /// Serde serialization failed before a request was admitted.
        SerdeSerialize,
        /// Serde deserialization failed after a value was retrieved.
        SerdeDeserialize,
        /// A value codec failed before a write was admitted.
        CodecEncode,
        /// A value codec failed after a value was retrieved.
        CodecDecode,
    }

    impl Error {
        /// Returns the stable category for this error.
        pub const fn kind(&self) -> ErrorKind {
            match self {
                Self::UnknownMutation { .. } => ErrorKind::UnknownMutation,
                Self::Core(_) => ErrorKind::Core,
                Self::UnsupportedSetOutcome => ErrorKind::UnsupportedSetOutcome,
                Self::SerdeSerialize(_) => ErrorKind::SerdeSerialize,
                Self::SerdeDeserialize(_) => ErrorKind::SerdeDeserialize,
                Self::CodecEncode(_) => ErrorKind::CodecEncode,
                Self::CodecDecode(_) => ErrorKind::CodecDecode,
            }
        }

        /// Returns whether this error reports an unknown mutation outcome.
        pub const fn is_unknown_mutation(&self) -> bool {
            matches!(self, Self::UnknownMutation { .. })
        }

        /// Returns the mutation whose outcome is unknown, if any.
        pub const fn mutation(&self) -> Option<Mutation> {
            match self {
                Self::UnknownMutation { operation } => Some(*operation),
                _ => None,
            }
        }
    }

    /// Result alias for client operations.
    pub type Result<T> = std::result::Result<T, Error>;

    impl SetOutcome {
        /// Returns whether this outcome created a new item.
        pub const fn is_created(self) -> bool {
            matches!(self, Self::Created)
        }

        /// Returns whether this outcome replaced an existing item.
        pub const fn is_replaced(self) -> bool {
            matches!(self, Self::Replaced)
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

    use core::ProtectedClient;

    /// A connected OpenKache client.
    ///
    /// Values use the OpenKache structured value format. Connections use the
    /// local development TLS profile described in [`Client::connect`].
    ///
    /// Cloning a client shares its connection and lifecycle state. Dropping a
    /// clone only releases that handle; dropping the final clone synchronously
    /// closes the transport as a best-effort abortive fallback. It does not
    /// wait for admitted operations to finish. Call [`Client::close`] and
    /// await it when graceful shutdown is required.
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
        ///
        /// # Arguments
        ///
        /// * `endpoint` - A `host:port` endpoint. IPv6 endpoints use
        ///   `[host]:port`.
        ///
        /// # Returns
        ///
        /// A connected [`Client`].
        ///
        /// # Errors
        ///
        /// Returns [`Error::Core`] when parsing the endpoint, connecting, or
        /// completing the TLS handshake fails.
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
        /// `Ok(Some(value))` when the item exists, or `Ok(None)` when it does
        /// not. A stored `Value::Null` is still `Some(Value::Null)`.
        ///
        /// # Errors
        ///
        /// Returns [`Error`] when the connection, protocol, key, or value
        /// operation fails.
        pub async fn get(&self, key: impl Into<TypedKey>) -> Result<Option<Value>> {
            self.gate0_client()
                .await?
                .get_structured(key)
                .await
                .map(|outcome| match outcome {
                    CoreGetOutcome::Found(value) => Some(value),
                    CoreGetOutcome::NotFound => None,
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

        /// Retrieves one value and decodes it into a Serde type.
        ///
        /// Deserialization is performed against the same lossless structured
        /// value model as [`Client::set`]. A missing key returns `None`. A
        /// stored null decodes as `Some(None)` when `T` is an `Option<U>`.
        ///
        /// # Arguments
        ///
        /// * `key` - A text, byte, or signed integer key convertible to
        ///   [`TypedKey`].
        ///
        /// # Returns
        ///
        /// `Ok(Some(value))` for a value that decodes into `T`, or `Ok(None)`
        /// when the key does not exist.
        ///
        /// # Errors
        ///
        /// Returns [`Error::SerdeDeserialize`] when the stored value cannot
        /// be represented by `T`, and [`Error::Core`] for transport,
        /// protocol, key, or value failures.
        pub async fn get_serde<T: DeserializeOwned>(
            &self,
            key: impl Into<TypedKey>,
        ) -> Result<Option<T>> {
            match self.get(key).await? {
                None => Ok(None),
                Some(value) => from_value(value)
                    .map(Some)
                    .map_err(|error| Error::SerdeDeserialize(error.to_string())),
            }
        }

        /// Serializes a Serde value and stores it with an unconditional
        /// write.
        ///
        /// Serde serialization completes before the request is admitted, so
        /// a serialization error cannot produce [`Error::UnknownMutation`].
        ///
        /// # Arguments
        ///
        /// * `key` - A text, byte, or signed integer key convertible to
        ///   [`TypedKey`].
        /// * `value` - The value to serialize.
        ///
        /// # Returns
        ///
        /// [`SetOutcome::Created`] for a new item or
        /// [`SetOutcome::Replaced`] when an existing item is overwritten.
        ///
        /// # Errors
        ///
        /// Returns [`Error::SerdeSerialize`] when `value` cannot be represented
        /// by the structured value model. Transport and protocol failures use
        /// the same errors as [`Client::set`].
        pub async fn set_serde<T: Serialize>(
            &self,
            key: impl Into<TypedKey>,
            value: T,
        ) -> Result<SetOutcome> {
            let value =
                to_value(&value).map_err(|error| Error::SerdeSerialize(error.to_string()))?;
            self.set(key, value).await
        }

        /// Retrieves one value and decodes it with an application codec.
        ///
        /// # Arguments
        ///
        /// * `key` - A text, byte, or signed integer key convertible to
        ///   [`TypedKey`].
        /// * `codec` - The application codec used to decode the stored
        ///   structured value.
        ///
        /// # Returns
        ///
        /// `Ok(Some(value))` when the item exists and decodes successfully, or
        /// `Ok(None)` when it does not.
        ///
        /// # Errors
        ///
        /// Returns [`Error::CodecDecode`] when `codec` rejects the stored
        /// value, or the same transport and protocol errors as [`Client::get`].
        pub async fn get_with<T, C>(&self, key: impl Into<TypedKey>, codec: &C) -> Result<Option<T>>
        where
            C: ValueCodecTrait<T>,
        {
            match self.get(key).await? {
                None => Ok(None),
                Some(value) => codec
                    .decode(value)
                    .map(Some)
                    .map_err(|error| Error::CodecDecode(error.to_string())),
            }
        }

        /// Encodes a native value with an application codec and stores it with
        /// an unconditional write.
        ///
        /// Encoding completes before request admission, so a codec failure
        /// cannot produce [`Error::UnknownMutation`].
        ///
        /// # Arguments
        ///
        /// * `key` - A text, byte, or signed integer key convertible to
        ///   [`TypedKey`].
        /// * `value` - The value to encode.
        /// * `codec` - The application codec used to encode `value`.
        ///
        /// # Returns
        ///
        /// [`SetOutcome::Created`] for a new item or
        /// [`SetOutcome::Replaced`] when an existing item is overwritten.
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
        ///
        /// # Arguments
        ///
        /// * `key` - A text, byte, or signed integer key convertible to
        ///   [`TypedKey`].
        ///
        /// # Returns
        ///
        /// `Ok(true)` when an item was removed, or `Ok(false)` when the key did
        /// not exist.
        ///
        /// # Errors
        ///
        /// Returns [`Error`] when the connection, protocol, key, or mutation
        /// operation fails.
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

        /// Gracefully and idempotently closes the shared client connection.
        ///
        /// This is the explicit shutdown path: it rejects new operations,
        /// waits for all operations already admitted to settle, and then
        /// releases the transport. Repeated or concurrent calls, including
        /// calls through clones, wait for the same terminal state. The call
        /// that performs shutdown reports any core close error; calls
        /// arriving after a completed shutdown return `Ok(())`.
        ///
        /// Dropping a [`Client`] cannot await this drain. Dropping the final
        /// clone instead performs a synchronous, best-effort abortive
        /// transport close that may interrupt admitted work. Await this
        /// method whenever graceful completion is required. If this future is
        /// canceled after it starts draining, it performs the same abortive
        /// fallback so later close callers cannot remain stuck waiting for a
        /// terminal state.
        ///
        /// # Returns
        ///
        /// `Ok(())` after the connection is closed.
        ///
        /// # Errors
        ///
        /// Returns [`Error::Core`] when closing the connection fails.
        pub async fn close(&self) -> Result<()> {
            self.inner.close().await.map_err(map_core_error)
        }
    }
}

pub use maintained::*;
