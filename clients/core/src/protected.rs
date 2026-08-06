//! Shared application-key and plaintext-value clients for language bindings.

use std::sync::Arc;
use std::time::Duration;

use crate::value::{Compression, Encryption, Value};
use crate::{
    AlpnPolicy, Certificate, ClientIdentity, ClientTimeouts, ConnectionState, DataProtection,
    DataProtectionKey, DeleteOutcome, Endpoint, GetOutcome, NamespaceDescriptor, NamespacePolicy,
    Result, RetryPolicy, ServerTrust, SetOptions, SetOutcome,
};
#[cfg(feature = "quic-compio")]
use crate::{LocalRawClient, LocalRawClientBuilder};
#[cfg(feature = "quic-quinn")]
use crate::{RawClient, RawClientBuilder};

struct ProtectionSettings {
    compression: Compression,
    encryption: Encryption,
    key: DataProtectionKey,
}

impl ProtectionSettings {
    fn new(key: DataProtectionKey) -> Self {
        Self {
            compression: Compression::Disabled,
            encryption: Encryption::Robust,
            key,
        }
    }

    fn finish(self) -> Result<Arc<DataProtection>> {
        DataProtection::with_profile(self.key, self.compression, self.encryption).map(Arc::new)
    }
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
            let contract = crate::contract::operation_contract(operation);
            match (contract.request_kind, contract.response_kind) {
                (
                    crate::contract::OperationRequestKind::Empty,
                    crate::contract::OperationResponseKind::Pong
                    | crate::contract::OperationResponseKind::Empty,
                )
                | (
                    crate::contract::OperationRequestKind::ApplicationValue,
                    crate::contract::OperationResponseKind::ApplicationValue,
                ) => {
                    self.raw
                        .execute_raw(operation, [], value, set_options)
                        .await
                }
                (
                    crate::contract::OperationRequestKind::ScopedItem,
                    crate::contract::OperationResponseKind::Value,
                ) => Ok(crate::operation_get_result(self.get(application_key).await?)),
                (
                    crate::contract::OperationRequestKind::ScopedItem,
                    crate::contract::OperationResponseKind::SetOutcome,
                ) => Ok(crate::operation_set_result(
                    self.set(application_key, value.as_ref().to_vec(), set_options)
                        .await?,
                )),
                (
                    crate::contract::OperationRequestKind::ScopedItem,
                    crate::contract::OperationResponseKind::DeleteOutcome,
                ) => Ok(crate::operation_delete_result(self.delete(application_key).await?)),
                (
                    crate::contract::OperationRequestKind::ScopedNamespace,
                    crate::contract::OperationResponseKind::StatsJson
                    | crate::contract::OperationResponseKind::Empty,
                ) => {
                    self.raw
                        .execute_raw(operation, [], value, set_options)
                        .await
                }
                _ => Err(crate::Error::configuration(
                    "operation",
                    "protocol operation is not available through the protected ABI",
                )),
            }
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
            let contract = crate::contract::operation_contract(operation);
            match (contract.request_kind, contract.response_kind) {
                (
                    crate::contract::OperationRequestKind::ScopedItem,
                    crate::contract::OperationResponseKind::Value,
                ) => {
                    let item_id = self.protection.item_id(application_key);
                    Ok(match self.raw.get_in_namespace(namespace_id, item_id).await? {
                        GetOutcome::Found(value) => {
                            let value = self.protection.open(item_id, value)?;
                            crate::operation_result(crate::contract::FfiResultKind::Value, value)
                        }
                        GetOutcome::NotFound => crate::operation_result(
                            crate::contract::FfiResultKind::NotFound,
                            Vec::new(),
                        ),
                    })
                }
                (
                    crate::contract::OperationRequestKind::ScopedItem,
                    crate::contract::OperationResponseKind::SetOutcome,
                ) => {
                    let item_id = self.protection.item_id(application_key.as_ref());
                    let value = self.protection.seal_owned(item_id, value.as_ref().to_vec())?;
                    Ok(crate::operation_set_result(
                        self.raw
                            .set_in_namespace(namespace_id, item_id, value, set_options)
                            .await?,
                    ))
                }
                (
                    crate::contract::OperationRequestKind::ScopedItem,
                    crate::contract::OperationResponseKind::DeleteOutcome,
                ) => {
                    let item_id = self.protection.item_id(application_key);
                    Ok(crate::operation_delete_result(
                        self.raw.delete_in_namespace(namespace_id, item_id).await?,
                    ))
                }
                (
                    crate::contract::OperationRequestKind::ScopedNamespace,
                    crate::contract::OperationResponseKind::StatsJson
                    | crate::contract::OperationResponseKind::Empty,
                ) => {
                    self.raw
                        .execute_scoped(operation, namespace_id, [], value, set_options)
                        .await
                }
                _ => Err(crate::Error::configuration(
                    "operation",
                    "protocol operation is not available through the namespace-scoped protected ABI",
                )),
            }
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

        /// Retrieves, authenticates, and decodes a value for arbitrary application key bytes.
        pub async fn get(&self, application_key: impl AsRef<[u8]>) -> Result<GetOutcome<Vec<u8>>> {
            match self.get_value(application_key).await? {
                GetOutcome::Found(Value::Raw(value)) => Ok(GetOutcome::Found(value)),
                GetOutcome::Found(Value::Json(_)) => {
                    Err(crate::value::Error::ExpectedRawValue.into())
                }
                GetOutcome::NotFound => Ok(GetOutcome::NotFound),
            }
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
        pub async fn get_value(
            &self,
            application_key: impl AsRef<[u8]>,
        ) -> Result<GetOutcome<Value>> {
            let item_id = self.protection.item_id(application_key);
            match self.raw.get(item_id).await? {
                GetOutcome::Found(value) => self
                    .protection
                    .decode(item_id, value)
                    .map(GetOutcome::Found),
                GetOutcome::NotFound => Ok(GetOutcome::NotFound),
            }
        }

        /// Protects and stores plaintext bytes for arbitrary application key bytes.
        pub async fn set(
            &self,
            application_key: impl AsRef<[u8]>,
            plaintext: Vec<u8>,
            options: SetOptions,
        ) -> Result<SetOutcome> {
            self.set_value(application_key, Value::Raw(plaintext), options)
                .await
        }

        /// Serializes, protects, and stores a core logical value.
        ///
        /// # Arguments
        ///
        /// * `application_key` - Exact application key bytes used for item ID derivation.
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
            application_key: impl AsRef<[u8]>,
            value: Value,
            options: SetOptions,
        ) -> Result<SetOutcome> {
            let item_id = self.protection.item_id(application_key);
            let value = self.protection.encode(item_id, value)?;
            self.raw.set(item_id, value, options).await
        }

        /// Deletes a value for arbitrary application key bytes.
        pub async fn delete(&self, application_key: impl AsRef<[u8]>) -> Result<DeleteOutcome> {
            self.raw
                .delete(self.protection.item_id(application_key))
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

    protected_client_methods!(LocalRawClient);
}
