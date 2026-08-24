// Generated from the OpenKache Smithy client contract. Do not edit.

fn smithy_require_kind(
    result: &openkache_client_core::OperationResult,
    expected: &[u32],
    operation: &str,
) -> std::result::Result<(), Error> {
    if expected.contains(&result.kind) {
        return Ok(());
    }
    Err(Error::Protocol(format!(
        "{operation} returned unexpected result kind {}",
        result.kind,
    )))
}







macro_rules! impl_smithy_api {
    ($client:ident) => {
        impl smithy::OpenKacheApi for $client {
            type Error = Error;

            async fn ping(
                &self,
                input: smithy::PingInput,
            ) -> std::result::Result<smithy::PingOutput, Self::Error> {
                let _ = &input;
                let result = $client::execute_unary(
                    self,
                    openkache_client_core::Opcode::Ping,
                    Vec::new(),
                )
                    .await?;
                smithy_require_kind(
                    &result,
                    &[openkache_client_core::contract::FFI_RESULT_OK],
                    "PING",
                )?;
                Ok(smithy::PingOutput { payload: result.payload })
            }

            async fn get(
                &self,
                input: smithy::GetInput,
            ) -> std::result::Result<smithy::GetOutput, Self::Error> {
                let result = $client::execute_scoped(
                    self,
                    openkache_client_core::Opcode::Get,
                    input.namespace_id,
                    input.item_id,
                    Vec::new(),
                    SetOptions::new(),
                )
                    .await?;
                smithy_require_kind(
                    &result,
                    &[
                        openkache_client_core::contract::FFI_RESULT_VALUE,
                        openkache_client_core::contract::FFI_RESULT_NOT_FOUND,
                    ],
                    "GET",
                )?;
                let value = if result.kind
                    == openkache_client_core::contract::FFI_RESULT_NOT_FOUND
                {
                    None
                } else {
                    Some(result.payload)
                };
                Ok(smithy::GetOutput { value: value })
            }

            async fn set(
                &self,
                input: smithy::SetInput,
            ) -> std::result::Result<smithy::SetOutput, Self::Error> {
                let options = smithy_set_options(
                    input.condition,
                    input.expiration_mode,
                    input.ttl_milliseconds,
                    input.eviction_mode,
                )?;
                let result = $client::execute_scoped(
                    self,
                    openkache_client_core::Opcode::Set,
                    input.namespace_id,
                    input.item_id,
                    input.value,
                    options,
                )
                    .await?;
                smithy_require_kind(
                    &result,
                    &[
                        openkache_client_core::contract::FFI_RESULT_CREATED,
                        openkache_client_core::contract::FFI_RESULT_REPLACED,
                        openkache_client_core::contract::FFI_RESULT_NOT_STORED,
                    ],
                    "SET",
                )?;
                let outcome = match result.kind {
                    openkache_client_core::contract::FFI_RESULT_CREATED => {
                        smithy::SetOutcome::Created
                    }
                    openkache_client_core::contract::FFI_RESULT_REPLACED => {
                        smithy::SetOutcome::Replaced
                    }
                    openkache_client_core::contract::FFI_RESULT_NOT_STORED => {
                        smithy::SetOutcome::NotStored
                    }
                    _ => unreachable!("smithy_require_kind validated SET result"),
                };
                Ok(smithy::SetOutput { outcome: outcome })
            }

            async fn delete(
                &self,
                input: smithy::DeleteInput,
            ) -> std::result::Result<smithy::DeleteOutput, Self::Error> {
                let result = $client::execute_scoped(
                    self,
                    openkache_client_core::Opcode::Delete,
                    input.namespace_id,
                    input.item_id,
                    [],
                    SetOptions::new(),
                )
                    .await?;
                smithy_require_kind(
                    &result,
                    &[
                        openkache_client_core::contract::FFI_RESULT_DELETED,
                        openkache_client_core::contract::FFI_RESULT_NOT_DELETED,
                    ],
                    "DELETE",
                )?;
                let deleted =
                    result.kind == openkache_client_core::contract::FFI_RESULT_DELETED;
                Ok(smithy::DeleteOutput { deleted: deleted })
            }

            async fn experimental_stats(
                &self,
                input: smithy::ExperimentalStatsInput,
            ) -> std::result::Result<smithy::ExperimentalStatsOutput, Self::Error> {
                let result = $client::execute_scoped(
                    self,
                    openkache_client_core::Opcode::ExperimentalStats,
                    input.namespace_id,
                    [],
                    [],
                    SetOptions::new(),
                )
                    .await?;
                smithy_require_kind(
                    &result,
                    &[openkache_client_core::contract::FFI_RESULT_VALUE],
                    "EXPERIMENTAL_STATS",
                )?;
                let json = String::from_utf8(result.payload).map_err(|error| {
                    Error::Protocol(format!("EXPERIMENTAL_STATS response is not UTF-8: {error}"))
                })?;
                Ok(smithy::ExperimentalStatsOutput { json: json })
            }

            async fn experimental_sync(
                &self,
                input: smithy::ExperimentalSyncInput,
            ) -> std::result::Result<smithy::ExperimentalSyncOutput, Self::Error> {
                let result = $client::execute_scoped(
                    self,
                    openkache_client_core::Opcode::ExperimentalSync,
                    input.namespace_id,
                    [],
                    [],
                    SetOptions::new(),
                )
                    .await?;
                smithy_require_kind(
                    &result,
                    &[openkache_client_core::contract::FFI_RESULT_OK],
                    "EXPERIMENTAL_SYNC",
                )?;
                Ok(smithy::ExperimentalSyncOutput)
            }

            async fn namespace_open(
                &self,
                input: smithy::NamespaceOpenInput,
            ) -> std::result::Result<smithy::NamespaceOpenOutput, Self::Error> {
                let policy = input
                    .policy
                    .map(|policy| smithy_namespace_policy(
                        policy.default_expiration,
                        policy.default_ttl_milliseconds,
                        policy.expiration_override,
                        policy.default_eviction,
                        policy.eviction_override,
                    ))
                    .transpose()?;
                let (descriptor, created) = $client::namespace_open_with_outcome(
                    self,
                    input.name.into_bytes(),
                    input.create_if_missing,
                    policy,
                )
                .await?;
                Ok(smithy::NamespaceOpenOutput {
                    descriptor: smithy_namespace_descriptor(descriptor),
                    created: created,
                })
            }

            async fn namespace_update_policy(
                &self,
                input: smithy::NamespaceUpdatePolicyInput,
            ) -> std::result::Result<smithy::NamespaceUpdatePolicyOutput, Self::Error> {
                let policy = smithy_namespace_policy(
                    input.policy.default_expiration,
                    input.policy.default_ttl_milliseconds,
                    input.policy.expiration_override,
                    input.policy.default_eviction,
                    input.policy.eviction_override,
                )?;
                let descriptor = $client::namespace_update_policy(
                    self,
                    input.namespace_id,
                    input.expected_revision,
                    policy,
                )
                .await?;
                Ok(smithy::NamespaceUpdatePolicyOutput {
                    descriptor: smithy_namespace_descriptor(descriptor),
                })
            }

            async fn namespace_delete(
                &self,
                input: smithy::NamespaceDeleteInput,
            ) -> std::result::Result<smithy::NamespaceDeleteOutput, Self::Error> {
                $client::namespace_delete(self, input.namespace_id, input.expected_revision)
                    .await?;
                Ok(smithy::NamespaceDeleteOutput)
            }
        }
    };
}

#[cfg(feature = "quic-quinn")]
impl_smithy_api!(RawClient);

#[cfg(feature = "quic-compio")]
impl_smithy_api!(LocalRawClient);
