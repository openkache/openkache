//! Shared application-key and plaintext-value clients for language bindings.

use std::sync::Arc;
use std::time::Duration;

use crate::key::{KeyBinding, KeyInput};
use crate::value::{Compression, Encryption, Value};
use crate::{
    AlpnPolicy, Certificate, ClientIdentity, ClientTimeouts, ConnectionState, DataProtection,
    DataProtectionKey, DeleteOutcome, Endpoint, GetOutcome, KeySpec, NamespaceDescriptor,
    NamespacePolicy, PortableKey, ResolvedKey, Result, RetryPolicy, ServerTrust, SetOptions,
    SetOutcome,
};
#[cfg(feature = "quic-compio")]
use crate::{LocalRawClient, LocalRawClientBuilder};
#[cfg(feature = "quic-quinn")]
use crate::{RawClient, RawClientBuilder};

struct ProtectionSettings {
    compression: Compression,
    encryption: Encryption,
    encryption_explicit: bool,
    key: Option<DataProtectionKey>,
    key_spec: KeySpec,
}

impl ProtectionSettings {
    fn new(key: DataProtectionKey) -> Self {
        Self::with_optional_key(Some(key))
    }

    fn unprotected() -> Self {
        Self::with_optional_key(None)
    }

    fn with_optional_key(key: Option<DataProtectionKey>) -> Self {
        Self {
            compression: Compression::Disabled,
            encryption: Encryption::Robust,
            encryption_explicit: false,
            key,
            key_spec: KeySpec::Bytes,
        }
    }

    fn finish(self) -> Result<Arc<DataProtection>> {
        let key_spec = self.key_spec;
        match self.key {
            Some(key) => if self.encryption == Encryption::Unprotected {
                DataProtection::unprotected(key_spec, self.compression)
            } else {
                DataProtection::with_profile_and_key_spec(
                    key,
                    key_spec,
                    self.compression,
                    self.encryption,
                )
            }
            .map(Arc::new),
            None => {
                if self.encryption_explicit && self.encryption != Encryption::Unprotected {
                    return Err(crate::Error::configuration(
                        "encryption",
                        "an encryption profile requires client_root_key",
                    ));
                }
                DataProtection::unprotected(key_spec, self.compression).map(Arc::new)
            }
        }
    }
}

fn require_key_binding(key: Option<&KeyBinding>) -> Result<&KeyBinding> {
    key.ok_or_else(|| {
        crate::Error::configuration("application_key", "protected operation requires a key")
    })
}

macro_rules! protected_builder_methods {
    ($builder:ident) => {
        impl $builder {
            /// Uses only the supplied server trust roots.
            pub fn server_trust(mut self, trust: ServerTrust) -> Self {
                self.raw = self.raw.server_trust(trust);
                self
            }

            /// Trusts one explicit CA or self-signed server certificate.
            pub fn trust_certificate(mut self, certificate: Certificate) -> Self {
                self.raw = self.raw.trust_certificate(certificate);
                self
            }

            /// Presents a mutual TLS client identity.
            pub fn client_identity(mut self, identity: ClientIdentity) -> Self {
                self.raw = self.raw.client_identity(identity);
                self
            }

            /// Offers protocol versions in descending order and enforces a minimum version.
            pub fn alpn_policy(mut self, policy: AlpnPolicy) -> Self {
                self.raw = self.raw.alpn_policy(policy);
                self
            }

            /// Sets connection and complete-request deadlines.
            pub fn timeouts(mut self, timeouts: ClientTimeouts) -> Self {
                self.raw = self.raw.timeouts(timeouts);
                self
            }

            /// Sets retry attempts for response-safe operations.
            pub fn retry_policy(mut self, retry: RetryPolicy) -> Self {
                self.raw = self.raw.retry_policy(retry);
                self
            }

            /// Bounds simultaneous request lanes on one QUIC connection.
            pub fn max_in_flight(mut self, maximum: usize) -> Self {
                self.raw = self.raw.max_in_flight(maximum);
                self
            }

            /// Selects a previously server-assigned namespace ID without resolving a name.
            pub fn namespace_id(mut self, namespace_id: u64) -> Self {
                self.raw = self.raw.namespace_id(namespace_id);
                self
            }

            /// Resolves this namespace name with `CreateIfMissing` during connection setup.
            pub fn namespace_name(mut self, namespace_name: impl AsRef<[u8]>) -> Self {
                self.raw = self.raw.namespace_name(namespace_name);
                self
            }

            /// Supplies the policy used if the configured namespace name is missing.
            pub fn namespace_policy(mut self, policy: NamespacePolicy) -> Self {
                self.raw = self.raw.namespace_policy(policy);
                self
            }

            /// Applies optional client-side compression before encryption.
            pub fn compression(mut self, compression: Compression) -> Self {
                self.protection.compression = compression;
                self
            }

            /// Selects the authenticated-encryption profile for stored values.
            ///
            /// # Arguments
            ///
            /// * `encryption` - Compact or Robust authenticated-encryption profile.
            ///
            /// # Returns
            ///
            /// This builder with the selected value protection profile.
            pub fn encryption(mut self, encryption: Encryption) -> Self {
                self.protection.encryption = encryption;
                self.protection.encryption_explicit = true;
                self
            }

            /// Selects the exact key type accepted by this formatted keyspace.
            pub fn key_spec(mut self, key_spec: KeySpec) -> Self {
                self.protection.key_spec = key_spec;
                self
            }
        }
    };
}

macro_rules! protected_client_methods {
    ($raw:ty) => {
        /// Borrows the exact-item-ID protocol client owned by this protected client.
        pub fn raw(&self) -> &$raw {
            &self.raw
        }

        /// Resolves neutral logical key bytes with this client's configured
        /// [`KeySpec`]. Language adapters use this boundary instead of
        /// carrying a second key-space implementation.
        #[doc(hidden)]
        pub fn resolve_logical_key(
            &self,
            logical_bytes: impl AsRef<[u8]>,
        ) -> Result<ResolvedKey> {
            self.protection
                .resolve_key_input(KeyInput::configured_logical(
                    logical_bytes.as_ref().to_owned(),
                ))
        }

        /// Verifies the connection and returns the complete request round-trip time.
        pub async fn ping(&self) -> Result<Duration> {
            self.raw.ping().await
        }

        /// Sends an application payload through the generic Smithy operation boundary.
        ///
        /// This compatibility helper is deprecated; generated adapters should use the generic
        /// operation result returned by [`Self::execute_operation`].
        #[deprecated(note = "use execute_operation for generated Smithy operations")]
        pub async fn execute_application(
            &self,
            operation: crate::Opcode,
            value: impl AsRef<[u8]>,
        ) -> Result<Vec<u8>> {
            self.raw
                .execute_raw(operation, [], value, SetOptions::new())
                .await
                .map(|result| result.payload)
        }

        /// Executes a generated Smithy operation with application-key and value protection.
        ///
        /// The Smithy operation contract selects the protected request path and returns the
        /// shared native result representation used by language adapters.
        pub async fn execute_operation(
            &self,
            operation: crate::Opcode,
            application_key: impl AsRef<[u8]>,
            value: impl AsRef<[u8]>,
            set_options: SetOptions,
        ) -> Result<crate::OperationResult> {
            self.execute_operation_typed(
                operation,
                PortableKey::Bytes(application_key.as_ref().to_vec()),
                value,
                set_options,
            )
            .await
        }

        /// Executes a generated Smithy operation for one typed portable key.
        ///
        /// This is the generic key boundary used by native language adapters.
        /// The key is converted, canonicalized, and bound to the namespace by
        /// the shared core; adapters never serialize key CBOR themselves.
        pub async fn execute_operation_typed(
            &self,
            operation: crate::Opcode,
            application_key: impl Into<PortableKey>,
            value: impl AsRef<[u8]>,
            set_options: SetOptions,
        ) -> Result<crate::OperationResult> {
            self.execute_operation_key_input(
                operation,
                KeyInput::portable(application_key),
                value,
                set_options,
            )
                .await
        }

        /// Executes an operation after resolving and binding its key through
        /// the shared core key boundary.
        pub(crate) async fn execute_operation_key_input(
            &self,
            operation: crate::Opcode,
            input: KeyInput,
            value: impl AsRef<[u8]>,
            set_options: SetOptions,
        ) -> Result<crate::OperationResult> {
            let binding = if crate::protocol::uses_compact_item_route(operation) {
                Some(self.resolve_and_bind_key(input).await?)
            } else {
                None
            };
            self.execute_operation_with_binding(operation, binding, value, set_options)
                .await
        }

        /// Executes a generated operation from its canonical ordered field
        /// sequence.
        ///
        /// The fields are already protocol-value bytes. This low-level
        /// boundary is intended for modeled batch/CAS shapes whose repeated
        /// identities cannot be represented by one logical application key;
        /// the shared raw executor still validates identity, framing, status,
        /// and response semantics from the generated descriptor.
        pub async fn execute_operation_fields(
            &self,
            operation: crate::Opcode,
            fields: Vec<Option<Vec<u8>>>,
        ) -> Result<crate::OperationResult> {
            self.raw.execute_fields(operation, fields).await
        }

        /// Dispatches a generated operation after key resolution.
        async fn execute_operation_with_binding(
            &self,
            operation: crate::Opcode,
            binding: Option<KeyBinding>,
            value: impl AsRef<[u8]>,
            set_options: SetOptions,
        ) -> Result<crate::OperationResult> {
            if crate::protocol::uses_compact_item_route(operation) {
                let binding = require_key_binding(binding.as_ref())?;
                if crate::protocol::compact_item_count(operation) != 1 {
                    return Err(crate::Error::configuration(
                        "application_key",
                        "generic protected execution requires one item_id field; use the raw field executor for repeated identities",
                    ));
                }
                return self
                    .raw
                    .execute_scoped(
                        operation,
                        binding.namespace_id,
                        binding.item_id,
                        value,
                        set_options,
                    )
                    .await;
            }
            if crate::protocol::uses_compact_namespace_route(operation) {
                let namespace_id = self.raw.ensure_namespace_id().await?;
                return self
                    .raw
                    .execute_scoped(operation, namespace_id, [], value, set_options)
                    .await;
            }
            self.raw
                .execute_raw(operation, [], value, set_options)
                .await
        }

        /// Executes a generated Smithy operation in an explicitly supplied namespace with
        /// application-key and value protection.
        pub async fn execute_operation_scoped(
            &self,
            operation: crate::Opcode,
            namespace_id: u64,
            application_key: impl AsRef<[u8]>,
            value: impl AsRef<[u8]>,
            set_options: SetOptions,
        ) -> Result<crate::OperationResult> {
            self.execute_operation_scoped_typed(
                operation,
                namespace_id,
                PortableKey::Bytes(application_key.as_ref().to_vec()),
                value,
                set_options,
            )
            .await
        }

        /// Executes a generated operation in an explicitly supplied namespace
        /// for one portable logical key.
        pub async fn execute_operation_scoped_typed(
            &self,
            operation: crate::Opcode,
            namespace_id: u64,
            application_key: impl Into<PortableKey>,
            value: impl AsRef<[u8]>,
            set_options: SetOptions,
        ) -> Result<crate::OperationResult> {
            let binding = if crate::protocol::uses_compact_item_route(operation) {
                Some(
                    self.protection
                        .bind_key_input(namespace_id, KeyInput::portable(application_key))?,
                )
            } else {
                None
            };
            if crate::protocol::uses_compact_item_route(operation) {
                let binding = require_key_binding(binding.as_ref())?;
                if crate::protocol::compact_item_count(operation) != 1 {
                    return Err(crate::Error::configuration(
                        "application_key",
                        "generic protected execution requires one item_id field; use the raw field executor for repeated identities",
                    ));
                }
                return self
                    .raw
                    .execute_scoped(operation, namespace_id, binding.item_id, value, set_options)
                    .await;
            }
            if crate::protocol::uses_compact_namespace_route(operation) {
                return self
                    .raw
                    .execute_scoped(operation, namespace_id, [], value, set_options)
                    .await;
            }
            self.raw
                .execute_raw(operation, [], value, set_options)
                .await
        }

        /// Returns the currently selected server-assigned namespace ID.
        pub fn namespace_id(&self) -> Option<u64> {
            self.raw.namespace_id()
        }

        /// Resolves a namespace name and optionally creates it.
        pub async fn namespace_open(
            &self,
            name: impl AsRef<[u8]>,
            create_if_missing: bool,
            policy: Option<NamespacePolicy>,
        ) -> Result<NamespaceDescriptor> {
            self.raw
                .namespace_open(name, create_if_missing, policy)
                .await
        }

        /// Replaces a namespace policy using its current revision.
        pub async fn namespace_update_policy(
            &self,
            namespace_id: u64,
            expected_revision: u64,
            policy: NamespacePolicy,
        ) -> Result<NamespaceDescriptor> {
            self.raw
                .namespace_update_policy(namespace_id, expected_revision, policy)
                .await
        }

        /// Deletes an empty namespace using its current revision.
        pub async fn namespace_delete(
            &self,
            namespace_id: u64,
            expected_revision: u64,
        ) -> Result<()> {
            self.raw
                .namespace_delete(namespace_id, expected_revision)
                .await
        }

        /// Retrieves, authenticates, and decodes a value for a portable key.
        pub async fn get(&self, key: impl Into<PortableKey>) -> Result<GetOutcome<Vec<u8>>> {
            let binding = self.resolve_and_bind_key(KeyInput::portable(key)).await?;
            self.get_raw_at_item_id(binding.namespace_id, binding.item_id)
                .await
        }

        /// Retrieves a formatted value in the core logical model.
        ///
        /// # Arguments
        ///
        /// * `application_key` - Exact application key bytes used for item ID derivation.
        ///
        /// # Returns
        ///
        /// The decoded value or a not-found outcome.
        ///
        /// # Errors
        ///
        /// Returns an error when transport, authentication, decompression, or deserialization
        /// fails.
        pub async fn get_value(&self, key: impl Into<PortableKey>) -> Result<GetOutcome<Value>> {
            let binding = self.resolve_and_bind_key(KeyInput::portable(key)).await?;
            self.get_value_at_item_id(binding.namespace_id, binding.item_id)
                .await
        }

        /// Retrieves a value when the adapter already owns canonical key bytes.
        pub async fn get_canonical_key(
            &self,
            canonical_key: impl AsRef<[u8]>,
        ) -> Result<GetOutcome<Value>> {
            self.get_value_key_input(KeyInput::canonical_in_space(
                canonical_key.as_ref().to_owned(),
            ))
            .await
        }

        async fn get_value_at_item_id(
            &self,
            namespace_id: u64,
            item_id: crate::ItemId,
        ) -> Result<GetOutcome<Value>> {
            match self.raw.get_in_namespace(namespace_id, item_id).await? {
                GetOutcome::Found(value) => self
                    .protection
                    .decode_in_namespace(namespace_id, item_id, value)
                    .map(GetOutcome::Found),
                GetOutcome::NotFound => Ok(GetOutcome::NotFound),
            }
        }

        async fn resolve_and_bind_key(
            &self,
            input: KeyInput,
        ) -> Result<KeyBinding> {
            let key = self.protection.resolve_key_input(input)?;
            let namespace_id = self.raw.ensure_namespace_id().await?;
            self.protection.bind_resolved_key(namespace_id, &key)
        }

        /// Retrieves a value after resolving and binding one logical key.
        ///
        /// Native adapters use this boundary for typed JSON operations so
        /// they do not carry [`ResolvedKey`] or repeat the resolve/bind
        /// sequence themselves.
        pub(crate) async fn get_value_key_input(
            &self,
            input: KeyInput,
        ) -> Result<GetOutcome<Value>> {
            let binding = self.resolve_and_bind_key(input).await?;
            self.get_value_at_item_id(binding.namespace_id, binding.item_id)
                .await
        }

        /// Stores a value after resolving and binding one logical key.
        pub(crate) async fn set_value_key_input(
            &self,
            input: KeyInput,
            value: Value,
            options: SetOptions,
        ) -> Result<SetOutcome> {
            let binding = self.resolve_and_bind_key(input).await?;
            self.set_value_at_binding(binding, value, options).await
        }

        /// Deletes a value after resolving and binding one logical key.
        pub(crate) async fn delete_key_input(
            &self,
            input: KeyInput,
        ) -> Result<DeleteOutcome> {
            let binding = self.resolve_and_bind_key(input).await?;
            self.delete_at_binding(binding).await
        }

        async fn bind_resolved_key(&self, key: &ResolvedKey) -> Result<KeyBinding> {
            let namespace_id = self.raw.ensure_namespace_id().await?;
            self.protection.bind_resolved_key(namespace_id, key)
        }

        async fn get_value_with_key(&self, key: &ResolvedKey) -> Result<GetOutcome<Value>> {
            let binding = self.bind_resolved_key(key).await?;
            self.get_value_at_item_id(binding.namespace_id, binding.item_id)
                .await
        }

        async fn get_raw_at_item_id(
            &self,
            namespace_id: u64,
            item_id: crate::ItemId,
        ) -> Result<GetOutcome<Vec<u8>>> {
            match self.get_value_at_item_id(namespace_id, item_id).await? {
                GetOutcome::Found(Value::Raw(value)) => Ok(GetOutcome::Found(value)),
                GetOutcome::Found(Value::Json(_)) => {
                    Err(crate::value::Error::ExpectedRawValue.into())
                }
                GetOutcome::NotFound => Ok(GetOutcome::NotFound),
            }
        }

        async fn get_raw_with_key(&self, key: &ResolvedKey) -> Result<GetOutcome<Vec<u8>>> {
            let binding = self.bind_resolved_key(key).await?;
            self.get_raw_at_item_id(binding.namespace_id, binding.item_id)
                .await
        }

        #[doc(hidden)]
        pub async fn get_resolved(
            &self,
            key: ResolvedKey,
        ) -> Result<GetOutcome<Vec<u8>>> {
            self.get_raw_with_key(&key).await
        }

        #[doc(hidden)]
        pub async fn get_value_resolved(
            &self,
            key: ResolvedKey,
        ) -> Result<GetOutcome<Value>> {
            self.get_value_with_key(&key).await
        }

        /// Protects and stores plaintext bytes for a portable key.
        pub async fn set(
            &self,
            key: impl Into<PortableKey>,
            plaintext: Vec<u8>,
            options: SetOptions,
        ) -> Result<SetOutcome> {
            self.set_value(key, Value::Raw(plaintext), options).await
        }

        #[doc(hidden)]
        pub async fn set_resolved(
            &self,
            key: ResolvedKey,
            plaintext: Vec<u8>,
            options: SetOptions,
        ) -> Result<SetOutcome> {
            self.set_value_resolved(key, Value::Raw(plaintext), options)
                .await
        }

        /// Serializes, protects, and stores a core logical value.
        ///
        /// # Arguments
        ///
        /// * `key` - Portable key value used for Item ID derivation.
        /// * `value` - Raw or logical JSON value to encode.
        /// * `options` - Existence condition and optional expiration.
        ///
        /// # Returns
        ///
        /// The server's set outcome.
        ///
        /// # Errors
        ///
        /// Returns an error when serialization, protection, transport, or the server operation
        /// fails.
        pub async fn set_value(
            &self,
            key: impl Into<PortableKey>,
            value: Value,
            options: SetOptions,
        ) -> Result<SetOutcome> {
            let binding = self.resolve_and_bind_key(KeyInput::portable(key)).await?;
            self.set_value_at_binding(binding, value, options).await
        }

        async fn set_value_with_key(
            &self,
            key: &ResolvedKey,
            value: Value,
            options: SetOptions,
        ) -> Result<SetOutcome> {
            let binding = self.bind_resolved_key(key).await?;
            self.set_value_at_binding(binding, value, options).await
        }

        async fn set_value_at_binding(
            &self,
            binding: KeyBinding,
            value: Value,
            options: SetOptions,
        ) -> Result<SetOutcome> {
            let value = self
                .protection
                .encode_in_namespace(binding.namespace_id, binding.item_id, value)?;
            self.raw
                .set_in_namespace(binding.namespace_id, binding.item_id, value, options)
                .await
        }

        #[doc(hidden)]
        pub async fn set_value_resolved(
            &self,
            key: ResolvedKey,
            value: Value,
            options: SetOptions,
        ) -> Result<SetOutcome> {
            self.set_value_with_key(&key, value, options).await
        }

        /// Stores a value when the adapter already owns canonical key bytes.
        pub async fn set_canonical_key(
            &self,
            canonical_key: impl AsRef<[u8]>,
            value: Value,
            options: SetOptions,
        ) -> Result<SetOutcome> {
            self.set_value_key_input(
                KeyInput::canonical_in_space(canonical_key.as_ref().to_owned()),
                value,
                options,
            )
            .await
        }

        /// Deletes a value for a portable key.
        pub async fn delete(&self, key: impl Into<PortableKey>) -> Result<DeleteOutcome> {
            let binding = self.resolve_and_bind_key(KeyInput::portable(key)).await?;
            self.delete_at_binding(binding).await
        }

        async fn delete_with_key(&self, key: &ResolvedKey) -> Result<DeleteOutcome> {
            let binding = self.bind_resolved_key(key).await?;
            self.delete_at_binding(binding).await
        }

        async fn delete_at_binding(&self, binding: KeyBinding) -> Result<DeleteOutcome> {
            self.raw
                .delete_in_namespace(binding.namespace_id, binding.item_id)
                .await
        }

        #[doc(hidden)]
        pub async fn delete_resolved(&self, key: ResolvedKey) -> Result<DeleteOutcome> {
            self.delete_with_key(&key).await
        }

        /// Deletes a value when the adapter already owns canonical key bytes.
        pub async fn delete_canonical_key(
            &self,
            canonical_key: impl AsRef<[u8]>,
        ) -> Result<DeleteOutcome> {
            self.delete_key_input(KeyInput::canonical_in_space(
                canonical_key.as_ref().to_owned(),
            ))
            .await
        }

        /// Returns server statistics as their JSON text.
        pub async fn stats(&self) -> Result<String> {
            self.raw.stats().await
        }

        /// Waits until prior mutations satisfy the server durability barrier.
        pub async fn sync(&self) -> Result<()> {
            self.raw.sync().await
        }

        /// Returns a best-effort state snapshot that does not guarantee the next request succeeds.
        pub fn connection_state(&self) -> ConnectionState {
            self.raw.connection_state()
        }

        /// Explicitly replaces the current connection without replaying an operation.
        pub async fn reconnect(&self) -> Result<()> {
            self.raw.reconnect().await
        }

        /// Permanently and idempotently closes this client instance.
        pub async fn close(&self) -> Result<()> {
            self.raw.close().await
        }
    };
}

#[cfg(feature = "quic-quinn")]
/// Shared protected client running on Tokio and Quinn.
#[derive(Clone)]
pub struct ProtectedClient {
    raw: RawClient,
    protection: Arc<DataProtection>,
}

#[cfg(feature = "quic-quinn")]
/// Connection and data-protection builder for a shared Tokio client.
pub struct ProtectedClientBuilder {
    raw: RawClientBuilder,
    protection: ProtectionSettings,
}

#[cfg(feature = "quic-quinn")]
protected_builder_methods!(ProtectedClientBuilder);

#[cfg(feature = "quic-quinn")]
impl ProtectedClientBuilder {
    /// Connects a client with mandatory application-key and value protection.
    pub async fn connect(self) -> Result<ProtectedClient> {
        let protection = self.protection.finish()?;
        let raw = self.raw.connect().await?;
        Ok(ProtectedClient { raw, protection })
    }
}

#[cfg(feature = "quic-quinn")]
impl ProtectedClient {
    /// Connects with mandatory data protection, system trust, and default client behavior.
    pub async fn connect(endpoint: &str, key: DataProtectionKey) -> Result<Self> {
        Self::builder(endpoint.parse()?, key).connect().await
    }

    /// Starts explicit shared client configuration.
    pub fn builder(endpoint: Endpoint, key: DataProtectionKey) -> ProtectedClientBuilder {
        ProtectedClientBuilder {
            raw: RawClient::builder(endpoint),
            protection: ProtectionSettings::new(key),
        }
    }

    /// Starts an explicitly unprotected client with namespace-bound Item IDs.
    pub fn builder_unprotected(endpoint: Endpoint) -> ProtectedClientBuilder {
        ProtectedClientBuilder {
            raw: RawClient::builder(endpoint),
            protection: ProtectionSettings::unprotected(),
        }
    }

    protected_client_methods!(RawClient);
}

#[cfg(feature = "quic-compio")]
/// Shared protected client confined to a Compio runtime.
#[derive(Clone)]
pub struct LocalProtectedClient {
    raw: LocalRawClient,
    protection: Arc<DataProtection>,
}

#[cfg(feature = "quic-compio")]
/// Connection and data-protection builder for a shared Compio client.
pub struct LocalProtectedClientBuilder {
    raw: LocalRawClientBuilder,
    protection: ProtectionSettings,
}

#[cfg(feature = "quic-compio")]
protected_builder_methods!(LocalProtectedClientBuilder);

#[cfg(feature = "quic-compio")]
impl LocalProtectedClientBuilder {
    /// Connects a Compio client with mandatory application-key and value protection.
    pub async fn connect(self) -> Result<LocalProtectedClient> {
        let protection = self.protection.finish()?;
        let raw = self.raw.connect().await?;
        Ok(LocalProtectedClient { raw, protection })
    }
}

#[cfg(feature = "quic-compio")]
impl LocalProtectedClient {
    /// Connects with mandatory data protection, system trust, and default Compio behavior.
    pub async fn connect(endpoint: &str, key: DataProtectionKey) -> Result<Self> {
        Self::builder(endpoint.parse()?, key).connect().await
    }

    /// Starts explicit shared Compio client configuration.
    pub fn builder(endpoint: Endpoint, key: DataProtectionKey) -> LocalProtectedClientBuilder {
        LocalProtectedClientBuilder {
            raw: LocalRawClient::builder(endpoint),
            protection: ProtectionSettings::new(key),
        }
    }

    /// Starts an explicitly unprotected client with namespace-bound Item IDs.
    pub fn builder_unprotected(endpoint: Endpoint) -> LocalProtectedClientBuilder {
        LocalProtectedClientBuilder {
            raw: LocalRawClient::builder(endpoint),
            protection: ProtectionSettings::unprotected(),
        }
    }

    protected_client_methods!(LocalRawClient);
}
