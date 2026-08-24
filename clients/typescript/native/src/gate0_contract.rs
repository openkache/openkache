//! Generated Gate 0 profile projection used by the private TypeScript adapter.
//!
//! The value-format selectors and default server name come from the generated
//! Smithy contract exposed by `openkache-client-core::contract`.  The
//! maintained TypeScript facade adds the fixed namespace and Item-ID root
//! required by the public Gate 0 contract.  Keeping the projection in one
//! package-private module prevents the Node-API entry points from growing
//! independent profile literals.

use openkache_client_core::contract::{
    CLIENT_DEFAULT_SERVER_NAME, VALUE_FORMAT_COMPRESSION_NONE, VALUE_FORMAT_ENCRYPTION_NONE,
    VALUE_FORMAT_SERIALIZATION_STRUCTURED,
};
use openkache_client_core::value::{Compression, Encryption};
use openkache_client_core::{AlpnPolicy, ClientRootKey, ServerTrust};

const ROOT_KEY_BYTES: [u8; 32] = [
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
    0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f,
];

/// The fixed profile consumed by the TypeScript Gate 0 adapter.
pub(crate) struct Gate0Profile {
    pub(crate) server_name: &'static str,
    pub(crate) namespace_id: u64,
    pub(crate) item_id_root: ClientRootKey,
    pub(crate) server_trust: ServerTrust,
    pub(crate) compression: Compression,
    pub(crate) encryption: Encryption,
    pub(crate) alpn: AlpnPolicy,
}

/// Builds one fresh profile for a native connection.
pub(crate) fn profile() -> Gate0Profile {
    // These generated selectors are intentionally referenced here even
    // though the core's structured-value methods apply them internally.  A
    // contract change must therefore update this projection before the
    // TypeScript adapter can be rebuilt.
    let _structured_selector = VALUE_FORMAT_SERIALIZATION_STRUCTURED;
    let _compression_selector = VALUE_FORMAT_COMPRESSION_NONE;
    let _encryption_selector = VALUE_FORMAT_ENCRYPTION_NONE;

    Gate0Profile {
        server_name: CLIENT_DEFAULT_SERVER_NAME,
        namespace_id: 1,
        item_id_root: ClientRootKey::from_bytes(ROOT_KEY_BYTES),
        server_trust: ServerTrust::Insecure,
        compression: Compression::Disabled,
        encryption: Encryption::Unprotected,
        alpn: AlpnPolicy::default(),
    }
}
