// Generated from the OpenKache Smithy contract. Do not edit.

/// QUIC application protocol identifier for wire protocol version 3.
pub const ALPN: &[u8] = b"openkache/3";
/// Bytes before the variable-length request lengths.
pub const REQUEST_FIXED_BYTES: usize = 2;
/// Bytes before the variable-length response payload length.
pub const RESPONSE_FIXED_BYTES: usize = 1;
/// Maximum bytes in one unsigned `vu128` accepted by this protocol.
pub const MAX_VARUINT_BYTES: usize = 9;
/// Bytes in every canonical item key carried by the protocol.
pub const ITEM_KEY_BYTES: usize = 32;
/// Absolute value or response payload ceiling representable by protocol v3.
pub const MAX_VALUE_BYTES: usize = 67_108_864;

const SET_TTL_FLAG: u8 = 0x01;
const SET_IF_ABSENT_FLAG: u8 = 0x02;
const SET_IF_PRESENT_FLAG: u8 = 0x04;

wire_enum! {
    /// Operations supported by protocol v3.
    pub enum Opcode {
        Ping = 0x01,
        Get = 0x02,
        Set = 0x03,
        Delete = 0x04,
        Stats = 0x05,
        Sync = 0x06,
    }
    unknown => UnknownOpcode
}

wire_enum! {
    /// Status returned in every protocol response.
    pub enum Status {
        Ok = 0x00,
        NotFound = 0x01,
        Created = 0x02,
        Replaced = 0x03,
        Deleted = 0x04,
        NotStored = 0x05,
        InvalidRequest = 0x40,
        UnsupportedOpcode = 0x41,
        TooLarge = 0x42,
        Overloaded = 0x43,
        Timeout = 0x44,
        Forbidden = 0x45,
        InternalError = 0x7f,
    }
    unknown => UnknownStatus
}
