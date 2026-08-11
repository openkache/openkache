//! Connection capability evaluation for modeled operations.
//!
//! Authorization is kept separate from request decoding and operation
//! behavior. The operation registry stores only a predicate, while this module
//! owns the connection-local capability representation and its generic
//! fail-closed evaluation.

use std::sync::Arc;

use super::operation_api::capability_id;

/// API-owned authorization predicate stored in an operation definition.
///
/// The generic dispatcher invokes this callback without interpreting a
/// capability string or operation name. Compatibility operations use the
/// predicates below; future API modules may provide their own policy function.
pub(super) type AuthorizationFn = fn(&AuthorizationContext) -> bool;

/// Capabilities attached to one authenticated connection.
///
/// This is intentionally a capability set rather than an administrator flag.
/// The current TLS adapter supplies the historical `administrator` capability,
/// while another authentication provider can construct the same boundary from
/// its own capability names without changing generated operation metadata or
/// dispatch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct AuthorizationContext {
    /// Capabilities are owned by the connection, not by generated operation
    /// metadata. This keeps the authorization boundary usable for dynamic
    /// identities (tenant grants, session tokens, or policy refreshes) while
    /// retaining cheap clone-by-reference semantics across request lanes.
    capabilities: Arc<[CapabilityIdentity]>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CapabilityIdentity {
    id: u64,
    name: Box<str>,
}

impl CapabilityIdentity {
    fn new(name: &str) -> Self {
        Self {
            id: capability_id(name),
            name: name.into(),
        }
    }
}

impl AuthorizationContext {
    pub(super) fn from_capabilities(capabilities: &[&str]) -> Self {
        let mut identities = capabilities
            .iter()
            .map(|capability| CapabilityIdentity::new(capability))
            .collect::<Vec<_>>();
        normalize_capabilities(&mut identities);
        Self {
            capabilities: identities.into(),
        }
    }

    #[allow(dead_code)]
    pub(super) fn from_owned_capabilities<I>(capabilities: I) -> Self
    where
        I: IntoIterator,
        I::Item: Into<Box<str>>,
    {
        let mut identities = capabilities
            .into_iter()
            .map(Into::<Box<str>>::into)
            .map(|name| CapabilityIdentity {
                id: capability_id(name.as_ref()),
                name,
            })
            .collect::<Vec<_>>();
        normalize_capabilities(&mut identities);
        Self {
            capabilities: identities.into(),
        }
    }

    pub(super) fn public() -> Self {
        Self::from_capabilities(&[])
    }

    pub(super) fn administrator() -> Self {
        Self::from_capabilities(&["administrator"])
    }

    fn permits(&self, capability: &str) -> bool {
        let id = capability_id(capability);
        self.capabilities
            .iter()
            .any(|entry| entry.id == id && entry.name.as_ref() == capability)
    }
}

fn normalize_capabilities(identities: &mut Vec<CapabilityIdentity>) {
    identities.sort_unstable_by(|left, right| {
        left.id
            .cmp(&right.id)
            .then_with(|| left.name.cmp(&right.name))
    });
    identities.dedup_by(|left, right| left.id == right.id && left.name == right.name);
}

pub(super) fn authorization_none(_authorization: &AuthorizationContext) -> bool {
    true
}

pub(super) fn authorization_administrator(authorization: &AuthorizationContext) -> bool {
    authorization.permits("administrator")
}

/// Applies the generated authorization capability at the generic server
/// boundary. Domain handlers only receive requests that have passed this
/// capability check, so permissions do not become opcode-specific branches.
///
/// Unknown capabilities fail closed. They remain usable by a future
/// authentication adapter as soon as it supplies the matching token; no
/// generated helper or operation-name branch is required.
pub(super) fn authorization_allowed(
    registration: &super::operation_api::ServerOperationRegistration,
    authorization: AuthorizationContext,
) -> bool {
    (registration.authorization)(&authorization)
}
