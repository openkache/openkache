/** Rust rendering for the server-owned operation adapter. */

import type { Wire_Contract } from "../wire_types"

function formatted_decimal(value: number): string {
  return value.toString().replace(/\B(?=(\d{3})+(?!\d))/g, "_")
}

/** Renders generic operation metadata for the server-owned adapter. */
export function render_rust_server_contract(contract: Wire_Contract): string {
  const operation_status_variants = contract.statuses
    .map((status) => `    ${status.name},`)
    .join("\n")
  const operation_status_wire_arms = contract.statuses
    .map((status) => `            Self::${status.name} => Status::${status.name},`)
    .join("\n")
  return `// Generated from the OpenKache Smithy operation contract. Do not edit.

use openkache_protocol::Status;
// The server consumes only the canonical wire projection.  Keep the
// Client-result/retry and execution-scope metadata belongs to the respective
// adapters; it is intentionally absent from this server contract surface.
pub use openkache_protocol::operation::{
    operation_registry, operation_wire_spec, wire_codec_kind,
    MAX_OPERATION_FIELDS, MAX_OPERATION_REQUEST_FIELDS,
    request_fields,
    OperationFieldPlan, OperationFramePolicy, OperationLayoutFraming,
    OperationWireSpec, WireCodecCardinality, WireCodecDescriptor, WireCodecKind,
    WireCodecLengthEncoding, WireCodecWidth, WIRE_CODEC_DESCRIPTORS,
    WIRE_CODEC_NAMES,
};
pub use openkache_protocol::{
    MAX_REQUEST_FRAME_BYTES, RequestFrameLayout as WireRequestLayout,
    RequestFrameStep as WireRequestStep, wire_request_layout,
};

/// Transport-neutral semantic status generated from the modeled vocabulary.
///
/// API behavior returns this enum. Only the response adapter projects it to
/// the protocol wire value, so request execution performs no string lookup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationStatus {
${operation_status_variants}
}

impl OperationStatus {
    /// Projects one semantic status at the wire-adapter boundary.
    pub const fn wire_status(self) -> Status {
        match self {
${operation_status_wire_arms}
        }
    }
}

/// Aggregate payload ceiling for generic request layouts.
///
/// Frame prefixes are added by the composed transport adapter, so this
/// constant does not encode a protocol-version-specific opcode or varuint
/// width.
pub const MAX_GENERIC_REQUEST_PAYLOAD_BYTES: usize =
    ${formatted_decimal(contract.max_value_bytes)};
`
}
