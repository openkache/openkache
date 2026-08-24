/// Maximum UTF-8 octets accepted in a namespace name.
pub const NAMESPACE_NAME_MAX_BYTES: usize = 255;

/// Width of the SET flags field.
pub const SET_FLAGS_BYTES: usize = 1;
pub const SET_CONDITION_MASK: u8 = 0x03;
pub const SET_CONDITION_ANY_BITS: u8 = 0x00;
pub const SET_IF_ABSENT_BITS: u8 = 0x01;
pub const SET_IF_PRESENT_BITS: u8 = 0x02;
pub const SET_CONDITION_RESERVED_BITS: u8 = 0x03;
pub const SET_EXPIRATION_MASK: u8 = 0x0c;
pub const SET_INHERIT_EXPIRATION_BITS: u8 = 0x00;
pub const SET_NO_EXPIRY_BITS: u8 = 0x04;
pub const SET_EXPLICIT_TTL_BITS: u8 = 0x08;
pub const SET_EXPIRATION_RESERVED_BITS: u8 = 0x0c;
pub const SET_EVICTION_MASK: u8 = 0x30;
pub const SET_INHERIT_EVICTION_BITS: u8 = 0x00;
pub const SET_EVICTABLE_BITS: u8 = 0x10;
pub const SET_EVICTION_PROTECTED_BITS: u8 = 0x20;
pub const SET_EVICTION_RESERVED_BITS: u8 = 0x30;
pub const SET_RESERVED_MASK: u8 = 0xc0;

/// Namespace-open flag fields.
pub const OPEN_FLAGS_BYTES: usize = 1;
pub const OPEN_CREATE_IF_MISSING: u8 = 0x01;
pub const OPEN_RESERVED_MASK: u8 = 0xfe;

/// Namespace-delete flag fields.
pub const DELETE_FLAGS_BYTES: usize = 1;
pub const DELETE_IF_EMPTY: u8 = 0x00;
pub const DELETE_MODE_MASK: u8 = 0x03;
pub const DELETE_RESERVED_MASK: u8 = 0xfc;

/// Namespace-policy flag fields.
pub const POLICY_FLAGS_BYTES: usize = 1;
pub const POLICY_DEFAULT_EXPIRATION_MASK: u8 = 0x03;
pub const POLICY_NO_EXPIRY: u8 = 0x00;
pub const POLICY_FIXED_TTL: u8 = 0x01;
pub const POLICY_DEFAULT_EXPIRATION_RESERVED_BITS: u8 = 0x03;
pub const POLICY_EXPIRATION_OVERRIDE: u8 = 0x04;
pub const POLICY_EVICTION_PROTECTED: u8 = 0x08;
pub const POLICY_EVICTION_OVERRIDE: u8 = 0x10;
pub const POLICY_RESERVED_MASK: u8 = 0xe0;
