//! Generated Gate 0 profile projection used by the private TypeScript adapter.
//!
//! Every value below is projected from the generated Smithy client contract
//! exposed by `openkache-client-core::contract`.  Keeping the projection in
//! one package-private module prevents the Node-API entry points from growing
//! independent profile literals.

use openkache_client_core::contract::{
    CLIENT_DEFAULT_SERVER_NAME, CLIENT_GATE0_ALPN_VERSION, CLIENT_GATE0_COMPRESSION,
    CLIENT_GATE0_ENCRYPTION, CLIENT_GATE0_ITEM_ID_ROOT, CLIENT_GATE0_NAMESPACE_ID,
    CLIENT_GATE0_VALUE_SELECTOR, VALUE_FORMAT_COMPRESSION_NONE, VALUE_FORMAT_ENCRYPTION_NONE,
    VALUE_FORMAT_ENCRYPTION_SHIFT, VALUE_FORMAT_SERIALIZATION_STRUCTURED,
};
use openkache_client_core::value::{Compression, Encryption};
use openkache_client_core::{AlpnPolicy, ClientRootKey, ServerTrust};

/// The fixed profile consumed by the TypeScript Gate 0 adapter.
pub(crate) struct Gate0Profile {
    pub(crate) server_name: &'static str,
    pub(crate) item_id_root: ClientRootKey,
    pub(crate) server_trust: ServerTrust,
    pub(crate) compression: Compression,
    pub(crate) encryption: Encryption,
    pub(crate) alpn: AlpnPolicy,
}

/// Builds one fresh profile for a native connection.
pub(crate) fn profile() -> Gate0Profile {
    validate_value_selector();

    Gate0Profile {
        server_name: CLIENT_DEFAULT_SERVER_NAME,
        item_id_root: ClientRootKey::from_bytes(CLIENT_GATE0_ITEM_ID_ROOT),
        server_trust: ServerTrust::Insecure,
        compression: gate0_compression(),
        encryption: gate0_encryption(),
        alpn: AlpnPolicy::from_versions(vec![CLIENT_GATE0_ALPN_VERSION], CLIENT_GATE0_ALPN_VERSION)
            .expect("generated Gate 0 ALPN profile must be valid"),
    }
}

/// Returns the generated namespace identity expected after lazy resolution.
///
/// The native client deliberately does not pass this to the core builder:
/// fresh servers must resolve their first namespace through the core's normal
/// lazy `NAMESPACE_OPEN` path.
pub(crate) const fn namespace_id() -> u64 {
    CLIENT_GATE0_NAMESPACE_ID
}

fn gate0_compression() -> Compression {
    match CLIENT_GATE0_COMPRESSION {
        VALUE_FORMAT_COMPRESSION_NONE => Compression::Disabled,
        selector => panic!("generated Gate 0 compression selector {selector} is unsupported"),
    }
}

fn gate0_encryption() -> Encryption {
    match CLIENT_GATE0_ENCRYPTION {
        VALUE_FORMAT_ENCRYPTION_NONE => Encryption::Unprotected,
        selector => panic!("generated Gate 0 encryption selector {selector} is unsupported"),
    }
}

fn validate_value_selector() {
    let expected = CLIENT_GATE0_ENCRYPTION
        | (CLIENT_GATE0_COMPRESSION << 2)
        | (VALUE_FORMAT_SERIALIZATION_STRUCTURED << VALUE_FORMAT_ENCRYPTION_SHIFT);
    assert_eq!(
        CLIENT_GATE0_VALUE_SELECTOR, expected,
        "generated Gate 0 value selector does not match its profile fields",
    );
}
