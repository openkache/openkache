/** Rust rendering for the canonical operation and primitive wire contracts. */

import {
  MAX_GENERATED_OPERATION_FIELDS,
  WIRE_CODEC_DESCRIPTORS,
  WIRE_CODEC_NAMES,
  type Wire_Contract,
  type Wire_Entry,
  type Wire_Operation,
  type Wire_Operation_Descriptor,
  type Wire_Operation_Field_Layout,
  type Wire_Operation_Field_Plan,
  type Wire_Response_Framing,
} from "../wire_types"
import {
  derive_wire_operation_descriptor,
  request_payload_bound,
  response_payload_bound,
} from "../wire_descriptor"
import { fixed_field_width, fixed_plan_width } from "../wire_layout"
import { render_rust_request_layout } from "./render_rust_request"

function formatted_decimal(value: number): string {
  return value.toString().replace(/\B(?=(\d{3})+(?!\d))/g, "_")
}

function formatted_byte(value: number): string {
  return `0x${value.toString(16).padStart(2, "0")}`
}

function wire_name(identifier: string): string {
  return identifier
    .replace(/([a-z0-9])([A-Z])/g, "$1_$2")
    .replace(/([A-Z]+)([A-Z][a-z])/g, "$1_$2")
    .toLowerCase()
}

function pascal_case(identifier: string): string {
  return identifier
    .split(/[^A-Za-z0-9]+/)
    .filter((part) => part.length > 0)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join("")
}

function rust_const_identifier(identifier: string): string {
  let value = wire_name(identifier)
    .replace(/[^a-z0-9_]/g, "_")
    .replace(/^([0-9])/, "_$1")
    .toUpperCase()
  if (value.length === 0) value = "_FIELD"
  return value
}

function rust_byte_string_literal(value: string): string {
  const bytes = new TextEncoder().encode(value)
  let literal = 'b"'
  for (const byte of bytes) {
    if (byte >= 0x20 && byte <= 0x7e && byte !== 0x22 && byte !== 0x5c) {
      literal += String.fromCharCode(byte)
    } else {
      literal += `\\x${byte.toString(16).padStart(2, "0")}`
    }
  }
  return `${literal}"`
}

function rust_string_literal(value: string): string {
  let literal = '"'
  for (const character of value) {
    switch (character) {
      case '"':
        literal += '\\"'
        break
      case "\\":
        literal += "\\\\"
        break
      case "\n":
        literal += "\\n"
        break
      case "\r":
        literal += "\\r"
        break
      case "\t":
        literal += "\\t"
        break
      default: {
        const code_point = character.codePointAt(0) ?? 0
        literal +=
          code_point < 0x20
            ? `\\u{${code_point.toString(16)}}`
            : character
        break
      }
    }
  }
  return `${literal}"`
}

function rust_wire_enum(
  name: string,
  documentation: string,
  entries: readonly Wire_Entry[],
  unknown_variant: string,
): string {
  const variants = entries
    .map((entry) => `        ${entry.name} = ${formatted_byte(entry.value)},`)
    .join("\n")
  const all_variants = entries.map((entry) => `        Self::${entry.name},`).join("\n")
  const name_literals = entries
    .map((entry) => `        ${rust_string_literal(entry.text ?? wire_name(entry.name))},`)
    .join("\n")
  const names = entries
    .map(
      (entry) =>
        `        Self::${entry.name} => ${rust_string_literal(entry.text ?? wire_name(entry.name))},`,
    )
    .join("\n")
  return `wire_enum! {
    /// ${documentation}
    pub enum ${name} {
${variants}
    }
    unknown => ${unknown_variant}
}

impl ${name} {
    /// Number of values assigned by the Smithy ${name} contract.
    pub const COUNT: usize = ${entries.length};

    /// Every assigned Smithy ${name} value in wire-value order.
    pub const ALL: [Self; Self::COUNT] = [
${all_variants}
    ];

    /// Stable lowercase Smithy names in wire-value order.
    pub const NAMES: [&'static str; Self::COUNT] = [
${name_literals}
    ];

    /// Zero-based position in the Smithy value-order arrays.
    ///
    /// Wire values are intentionally allowed to be sparse. Callers that use
    /// an enum as an array index must use this generated position instead of
    /// the wire discriminant.
    pub const fn index(self) -> usize {
        match self {
${entries
  .map((entry, index) => `        Self::${entry.name} => ${index},`)
  .join("\n")}
        }
    }

    /// Stable lowercase Smithy name for this assigned value.
    pub const fn name(self) -> &'static str {
        match self {
${names}
        }
    }

    /// Resolves a generated Smithy name at an API adapter boundary.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
${entries
  .map((entry) => `            ${rust_string_literal(entry.text ?? wire_name(entry.name))} => Some(Self::${entry.name}),`)
  .join("\n")}
            _ => None,
        }
    }
}`
}

/**
 * Renders the canonical operation contract artifact.
 *
 * The protocol crate owns this artifact so server and client adapters consume
 * one field/framing/codec registry. Client retry/result metadata is rendered
 * by the client generator, and compatibility route metadata is rendered
 * separately by `compatibility_v1_renderer.ts`. `codec_kind_path` keeps the
 * generated module usable both inside the protocol crate and from the
 * server-owned compatibility adapter.
 */
export function render_rust_operation_contract(
  contract: Wire_Contract,
  codec_kind_path = "openkache_protocol::codec::CodecKind",
): string {
  const operations = contract.operations
  if (operations === undefined) return ""
  const max_request_fields = operations.reduce(
    (maximum, operation) => Math.max(
      maximum,
      operation.contract.request_plan?.length ?? 0,
    ),
    0,
  )
  const max_response_fields = operations.reduce(
    (maximum, operation) => Math.max(
      maximum,
      operation.contract.response_plan?.length ?? 0,
    ),
    0,
  )
  const max_operation_fields = Math.max(max_request_fields, max_response_fields)
  if (
    max_request_fields > MAX_GENERATED_OPERATION_FIELDS ||
    max_response_fields > MAX_GENERATED_OPERATION_FIELDS
  ) {
    throw new Error(
      `generated operation field plans exceed ${MAX_GENERATED_OPERATION_FIELDS} fields; ` +
        "use a bounded/streaming shape",
    )
  }
  const status_variant = (status: string): string => {
    const entry = contract.statuses.find(
      (candidate) =>
        candidate.name === status ||
        candidate.text === status ||
        wire_name(candidate.name) === status,
    )
    if (entry === undefined) {
      throw new Error(`operation metadata references unknown status ${status}`)
    }
    return entry.name
  }
  const descriptor = (operation: Wire_Operation): Wire_Operation_Descriptor =>
    derive_wire_operation_descriptor(operation.contract)
  const request_framing = (operation: Wire_Operation): string =>
    descriptor(operation).request_framing
  const request_layout = (operation: Wire_Operation): string =>
    descriptor(operation).request_layout
  const response_framing = (operation: Wire_Operation): Wire_Response_Framing =>
    descriptor(operation).response_framing
  const response_layout = (operation: Wire_Operation): string =>
    descriptor(operation).response_layout
  const status_slice = (statuses: readonly string[]): string =>
    `&[${statuses
      .map((status) => `Status::${status_variant(status)}`)
      .join(", ")}]`
  const plan_slice = (
    fields: readonly Wire_Operation_Field_Plan[] | undefined,
  ): string => {
    const plan = fields ?? []
    return `&[${plan
      .map((field, field_index, plan) => {
        let parent_index = "usize::MAX"
        for (let candidate = field_index - 1; candidate >= 0; candidate -= 1) {
          const parent = plan[candidate]
          if (
            parent.path.length < field.path.length &&
            parent.path.every((part, index) => field.path[index] === part)
          ) {
            parent_index = String(candidate)
            break
          }
        }
        return `OperationFieldPlan { index: ${field.index}, role: ${rust_string_literal(field.role)}, required: ${field.required}, parent_index: ${parent_index}, encoded_width: ${formatted_decimal(fixed_field_width(field) ?? 0)}, shape: ${rust_string_literal(field.shape)}, path: &[${field.path
          .map(rust_string_literal)
          .join(", ")}], codecs: &[${(field.codecs ?? [])
          .map(rust_string_literal)
          .join(", ")}], nested_codecs: &[${(field.nested_codecs ?? [])
          .map(rust_string_literal)
          .join(", ")}], nested_widths: &[${(field.nested_widths ?? [])
          .map((width) => width === undefined ? "0" : String(width))
          .join(", ")}], nested_enum_values: &[${(field.nested_enum_values ?? [])
          .map((values) => `&[${values.map(rust_string_literal).join(", ")}]`)
          .join(", ")}], nested_union_tags: &[${(field.nested_union_tags ?? [])
          .map((values) => `&[${values.join(", ")}]`)
          .join(", ")}], union_tags: &[${(field.union_tags ?? []).join(", ")}], enum_values: &[${(field.enum_values ?? [])
          .map(rust_string_literal)
          .join(", ")}] }`
      })
      .join(", ")}]`
  }
  const field_index_modules = (
    direction: "request" | "response",
  ): string => {
    const operation_modules = operations.map((operation) => {
      const fields = direction === "request"
        ? operation.contract.request_plan ?? []
        : operation.contract.response_plan ?? []
      const used_names = new Set<string>()
      const constants = fields.map((field, index) => {
        const name = rust_const_identifier(field.path.join("_"))
        if (used_names.has(name)) {
          throw new Error(
            `${operation.name} ${direction} fields produce duplicate Rust handle ${name}`,
          )
        }
        used_names.add(name)
        return `        /// Numeric index for ${field.path.join(".")}.
        pub const ${name}: usize = ${index};`
      })
      const module_name = `op_${wire_name(operation.name).replace(/[^a-z0-9_]/g, "_")}`
      return `    /// Generated field handles for ${operation.name}.
    pub mod ${module_name} {
${constants.join("\n")}
    }`
    })
    return `/// Generated numeric ${direction} field handles.
///
/// API-owned bindings import their operation module directly. The generic
/// dispatcher sees only indexes and never scans model role strings.
pub mod ${direction}_fields {
${operation_modules.join("\n")}
}
`
  }
  // Snapshot plans before evaluating any derived descriptors. Keeping the
  // renderer on immutable IR data avoids a helper accidentally replacing a
  // plan while the generated template is being assembled.
  const operation_plans = new Map(
    operations.map((operation) => [
      operation.name,
      {
        request_plan: operation.contract.request_plan,
        response_plan: operation.contract.response_plan,
      },
    ]),
  )
  const optional_value_codec = (
    layout: Wire_Operation_Field_Layout,
  ): string => {
    switch (layout) {
      case "optional_values":
        return `Some(match OptionalValueCodec::new(${formatted_decimal(contract.v1.optional_value_length_bytes ?? 4)}, ${formatted_decimal(contract.v1.optional_value_missing ?? 0xffff_ffff)}) {
                Ok(codec) => codec,
                Err(_) => panic!("generated optional-value wire constants are invalid"),
            })`
      case "empty":
      case "opaque":
      case "sequence":
      case "dense":
        return "None"
      default: {
        const exhaustive_layout: never = layout
        throw new Error(`unsupported operation field layout ${exhaustive_layout}`)
      }
    }
  }
  const wire_metadata = operations
    .map((operation) => {
      const plans = operation_plans.get(operation.name)!
      const request_plan = plans.request_plan
      const response_plan = plans.response_plan
      const operation_descriptor = descriptor(operation)
      return `    OperationWireSpec {
        request: OperationLayoutPlan {
            framing: OperationLayoutFraming::${pascal_case(operation_descriptor.request_framing)},
            frame: OperationFramePolicy::${pascal_case(operation_descriptor.request_frame)},
            layout: OperationFieldLayout::${pascal_case(operation_descriptor.request_layout)},
            optional_value_codec: ${optional_value_codec(operation_descriptor.request_layout)},
            fields: ${plan_slice(request_plan)},
            exact_width: ${formatted_decimal(fixed_plan_width(request_plan) ?? 0)},
            max_width: ${formatted_decimal(request_payload_bound(contract, operation))},
            opaque_aggregate: false,
        },
        response: OperationLayoutPlan {
            framing: OperationLayoutFraming::${pascal_case(operation_descriptor.response_framing)},
            frame: OperationFramePolicy::${pascal_case(operation_descriptor.response_frame)},
            layout: OperationFieldLayout::${pascal_case(operation_descriptor.response_layout)},
            optional_value_codec: ${optional_value_codec(operation_descriptor.response_layout)},
            fields: ${plan_slice(response_plan)},
            exact_width: ${formatted_decimal(fixed_plan_width(response_plan) ?? 0)},
            max_width: ${formatted_decimal(response_payload_bound(contract, operation))},
            opaque_aggregate: ${operation.contract.opaque_aggregate === true},
        },
        response_payload_bound: ${formatted_decimal(response_payload_bound(contract, operation))},
        success_statuses: ${status_slice(operation.contract.success_statuses)},
        error_statuses: ${status_slice(operation.contract.error_statuses)},
    }`
    })
    .join(",\n")
  const registry_metadata = operations
    .map(
      (operation, index) =>
        `    OperationRegistryEntry { opcode: Opcode::${operation.name}, wire: OPERATION_WIRE_SPECS[${index}] },`,
    )
    .join("\n")
  const codec_names = [
    ...new Set(
      operations.flatMap((operation) => [
        ...(operation.contract.request_plan ?? []),
        ...(operation.contract.response_plan ?? []),
      ]).flatMap((field) => [
        ...(field.codecs ?? []),
        ...(field.nested_codecs ?? []),
      ]),
    ),
  ].sort()
  const codec_name_metadata = codec_names
    .map(rust_string_literal)
    .join(", ")
  const supported_codec_name_metadata = WIRE_CODEC_NAMES
    .map(rust_string_literal)
    .join(", ")
  const codec_descriptor_metadata = WIRE_CODEC_NAMES
    .map((name) => {
      const descriptor = WIRE_CODEC_DESCRIPTORS[name]
      const kind = name
        .split("_")
        .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
        .join("")
      const width = descriptor.width === "fixed"
        ? `WireCodecWidth::Fixed(${descriptor.min_width})`
        : "WireCodecWidth::Variable"
      const cardinality = {
        scalar: "Scalar",
        repeated: "Repeated",
        associative: "Associative",
        tagged: "Tagged",
      }[descriptor.cardinality]
      const length_encoding = {
        none: "None",
        byte: "Byte",
        varuint: "VarUInt",
      }[descriptor.length_encoding]
      return `    WireCodecDescriptor { name: ${rust_string_literal(name)}, kind: WireCodecKind::${kind}, width: ${width}, min_width: ${descriptor.min_width}, max_width: ${descriptor.max_width ?? "usize::MAX"}, borrowable: ${descriptor.borrowable}, cardinality: WireCodecCardinality::${cardinality}, length_encoding: WireCodecLengthEncoding::${length_encoding}, container: ${descriptor.container}, recursive: ${descriptor.recursive} },`
    })
    .join("\n")
  const codec_kind_metadata = WIRE_CODEC_NAMES
    .map((name) => {
      const kind = name
        .split("_")
        .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
        .join("")
      return `        ${rust_string_literal(name)} => Some(${codec_kind_path}::${kind}),`
    })
    .join("\n")
  const request_wire_contract = render_rust_request_layout(contract)
  return `/// Maximum number of ordered request fields in any modeled operation.
///
/// Server operation views use this generated bound to reject unbounded shapes.
/// Runtime metadata uses bounded inline storage and spills only when a valid
/// model actually exceeds its inline capacity.
pub const MAX_OPERATION_REQUEST_FIELDS: usize = ${max_request_fields};

/// Maximum ordered fields in either request or response plans.
///
/// Client and server views use this generated bound for offset-storage
/// validation; ordinary plans keep their offsets inline and larger valid plans
/// spill through the same bounded representation.
pub const MAX_OPERATION_FIELDS: usize = ${max_operation_fields};

${request_wire_contract}

${field_index_modules("request")}
${field_index_modules("response")}

/// Generic response payload framing selected by the modeled operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationResponseFraming {
    Empty,
    Opaque,
    OptionalValues,
    FieldSequence,
}

/// Generic request framing consumed by transport-neutral executors.
///
/// Historical protocol-v1 routes are handled by adapters; generic server/client
/// code only needs this byte-shape class.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationRequestFraming {
    Empty,
    Opaque,
    OrderedFields,
}

/// Generic field payload layout selected by the generated shape plan.
///
/// Empty and opaque are explicit non-field layouts. Dense is used only for
/// all-required flattened fixed-width plans. Sequence is the general fallback
/// for optional, variable, repeated, and nested values. OptionalValues is an
/// explicit fixed presence-table layout selected by the operation descriptor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationFieldLayout {
    Empty,
    Opaque,
    Sequence,
    Dense,
    OptionalValues,
}

/// Generic frame policy selected by the same shape plan as the payload
/// layout. Fixed-body framing is safe only when the generated plan has an
/// exact width; all variable shapes remain length-delimited.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationFramePolicy {
    LengthDelimited,
    FixedBody,
}

/// Framing-neutral view of one request or response layout plan.
///
/// The fields and widths are generated together with the frame policy so
/// parsers, encoders, and clients cannot independently rediscover the shape.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OperationLayoutPlan {
    pub framing: OperationLayoutFraming,
    pub frame: OperationFramePolicy,
    pub layout: OperationFieldLayout,
    /// Layout-owned optional-value framing, absent for every other layout.
    pub optional_value_codec: Option<OptionalValueCodec>,
    pub fields: &'static [OperationFieldPlan],
    pub exact_width: usize,
    pub max_width: usize,
    /// An API-owned adapter explicitly permits a composite opaque aggregate.
    pub opaque_aggregate: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationLayoutFraming {
    Empty,
    Opaque,
    OrderedFields,
    OptionalValues,
    FieldSequence,
}

/// One ordered field in a generated request or response plan.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OperationFieldPlan {
    pub index: usize,
    pub role: &'static str,
    pub required: bool,
    pub parent_index: usize,
    pub encoded_width: usize,
    pub shape: &'static str,
    pub path: &'static [&'static str],
    pub codecs: &'static [&'static str],
    pub nested_codecs: &'static [&'static str],
    /// Fixed widths known for nested codecs; zero means variable/unknown.
    pub nested_widths: &'static [usize],
    pub nested_enum_values: &'static [&'static [&'static str]],
    pub nested_union_tags: &'static [&'static [u8]],
    pub union_tags: &'static [u8],
    pub enum_values: &'static [&'static str],
}

/// Canonical wire framing, field, and status metadata for one operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OperationWireSpec {
    pub request: OperationLayoutPlan,
    pub response: OperationLayoutPlan,
    /// Conservative maximum response payload bytes derived from the output shape.
    pub response_payload_bound: usize,
    pub success_statuses: &'static [Status],
    pub error_statuses: &'static [Status],
}

impl OperationWireSpec {
    /// Returns the request framing enum derived from the canonical layout.
    pub const fn request_framing(self) -> OperationRequestFraming {
        match self.request.framing {
            OperationLayoutFraming::Empty => OperationRequestFraming::Empty,
            OperationLayoutFraming::Opaque => OperationRequestFraming::Opaque,
            OperationLayoutFraming::OrderedFields
            | OperationLayoutFraming::FieldSequence
            | OperationLayoutFraming::OptionalValues => OperationRequestFraming::OrderedFields,
        }
    }

    /// Returns the response framing enum derived from the canonical layout.
    pub const fn response_framing(self) -> OperationResponseFraming {
        match self.response.framing {
            OperationLayoutFraming::Empty => OperationResponseFraming::Empty,
            OperationLayoutFraming::Opaque => OperationResponseFraming::Opaque,
            OperationLayoutFraming::OptionalValues => OperationResponseFraming::OptionalValues,
            OperationLayoutFraming::FieldSequence
            | OperationLayoutFraming::OrderedFields => OperationResponseFraming::FieldSequence,
        }
    }

    pub const fn request_layout(self) -> OperationFieldLayout {
        self.request.layout
    }

    pub const fn response_layout(self) -> OperationFieldLayout {
        self.response.layout
    }

    pub const fn request_plan(self) -> &'static [OperationFieldPlan] {
        self.request.fields
    }

    pub const fn response_plan(self) -> &'static [OperationFieldPlan] {
        self.response.fields
    }
}

/// Returns the canonical wire spec for one protocol operation.
pub const fn operation_wire_spec(opcode: Opcode) -> OperationWireSpec {
    OPERATION_WIRE_SPECS[opcode.index()]
}

/// Generated operation registry entry used by server bind-time validation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OperationRegistryEntry {
    pub opcode: Opcode,
    pub wire: OperationWireSpec,
}

/// Wire-only operation descriptors in opcode order.
pub const OPERATION_WIRE_SPECS: [OperationWireSpec; Opcode::COUNT] = [
${wire_metadata}
];

/// Dense opcode-indexed registry used by server bind-time validation.
pub const OPERATION_REGISTRY: [OperationRegistryEntry; Opcode::COUNT] = [
${registry_metadata}
];

/// Codec identifiers supported by the protocol adapters.
///
/// This is emitted from the same registry that language generators consume;
/// it is deliberately separate from the operation-local list below so a
/// malformed model cannot make an unknown codec appear supported merely by
/// mentioning it in a field plan.
pub const WIRE_CODEC_NAMES: &[&'static str] = &[${supported_codec_name_metadata}];

/// Canonical codec validator kinds shared by server validation and adapters.
///
/// The identifier remains an open string at the model boundary, while the
/// generated kind selects the reusable semantic adapter. Adding an operation
/// that reuses a codec therefore does not add a server branch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WireCodecKind {
    BoolU8,
    Enum,
    F64Be,
    I32Be,
    List,
    Map,
    PackedF64Be,
    RawBytes,
    U64Be,
    Union,
    Utf8,
}

/// Canonical codec shape metadata shared by server validation and adapters.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WireCodecWidth {
    Fixed(usize),
    Variable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WireCodecCardinality {
    Scalar,
    Repeated,
    Associative,
    Tagged,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WireCodecLengthEncoding {
    None,
    Byte,
    VarUInt,
}

/// Recursive codec proof metadata used by layout planners and adapters.
///
/// \`max_width == usize::MAX\` means that the codec has no finite bound at this
/// layer. A container is dense-safe only after a future shape descriptor
/// supplies fixed cardinality and child widths; the base codec registry alone
/// intentionally cannot make that claim.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WireCodecDescriptor {
    pub name: &'static str,
    pub kind: WireCodecKind,
    pub width: WireCodecWidth,
    pub min_width: usize,
    pub max_width: usize,
    pub borrowable: bool,
    pub cardinality: WireCodecCardinality,
    pub length_encoding: WireCodecLengthEncoding,
    pub container: bool,
    pub recursive: bool,
}

pub const WIRE_CODEC_DESCRIPTORS: &[WireCodecDescriptor] = &[
${codec_descriptor_metadata}
];

/// Resolves a generated codec identifier to the shared protocol kind.
///
/// Server field validation calls this adapter directly; operation handlers do
/// not need a server-local forwarding wrapper for generic codec traversal.
pub fn wire_codec_kind(
    name: &str,
) -> Option<${codec_kind_path}> {
    match name {
${codec_kind_metadata}
        _ => None,
    }
}

/// Codec identifiers required by the generated operation plans.
///
/// This list is the model-owned support surface shared by server validation
/// and language-specific adapters. An adapter may implement only a subset,
/// but it must fail generation or bind-time validation explicitly instead of
/// silently treating a declared codec as opaque bytes.
pub const OPERATION_CODEC_NAMES: &[&'static str] = &[${codec_name_metadata}];

/// Returns the static generated operation registry.
pub const fn operation_registry() -> &'static [OperationRegistryEntry; Opcode::COUNT] {
    &OPERATION_REGISTRY
}

`
}

/**
 * Renders only the request metadata needed to find a v1 frame boundary.
 *
 * This is intentionally separate from the semantic operation contract below.
 * The server runtime must not inspect response meanings, retry policy, or
 * server behavior just to read a request from a stream. Those fields remain
 * available to the generated server adapter, which owns this request-only
 * descriptor.
 */
function max_response_frame_bytes_for_contract(contract: Wire_Contract): number {
  const operations = contract.operations
  const maximum_payload = operations === undefined || operations.length === 0
    ? contract.max_value_bytes
    : Math.max(
      ...operations.map((operation) => response_payload_bound(contract, operation)),
    )
  return (
    contract.v1.status_bytes +
    contract.v1.max_varuint_bytes +
    maximum_payload
  )
}

/** Renders protocol v1 identifiers and common wire primitives. */
export function render_rust_wire(contract: Wire_Contract): string {
  const v1 = contract.v1
  return `// Generated from the OpenKache Smithy wire contract. Do not edit.

/// QUIC application protocol identifier for wire protocol version 1.
pub const ALPN: &[u8] = ${rust_byte_string_literal(v1.alpn)};
/// Bytes occupied by the request opcode.
pub const OPCODE_BYTES: usize = ${formatted_decimal(v1.opcode_bytes)};
/// Bytes occupied by the response status.
pub const STATUS_BYTES: usize = ${formatted_decimal(v1.status_bytes)};
/// Bytes before the variable-length request lengths.
pub const REQUEST_FIXED_BYTES: usize = ${formatted_decimal(v1.request_fixed_bytes)};
/// Bytes before the variable-length response payload length.
pub const RESPONSE_FIXED_BYTES: usize = ${formatted_decimal(v1.response_fixed_bytes)};
/// Minimum bytes in one canonical unsigned \`vu128\`.
pub const MIN_VARUINT_BYTES: usize = ${formatted_decimal(v1.min_varuint_bytes)};
/// Maximum bytes in one unsigned \`vu128\` accepted by this protocol.
pub const MAX_VARUINT_BYTES: usize = ${formatted_decimal(v1.max_varuint_bytes)};
/// Bytes in every canonical item ID carried by the protocol.
pub const ITEM_ID_BYTES: usize = ${formatted_decimal(contract.item_id_bytes)};
/// Absolute value or response payload ceiling representable by protocol v1.
pub const MAX_VALUE_BYTES: usize = ${formatted_decimal(contract.max_value_bytes)};
/// Conservative maximum complete response frame size for protocol v1.
pub const MAX_RESPONSE_FRAME_BYTES: usize =
    ${formatted_decimal(max_response_frame_bytes_for_contract(contract))};
/// Bytes in every namespace ID and namespace revision.
pub const NAMESPACE_ID_BYTES: usize = ${formatted_decimal(v1.namespace_id_bytes)};
pub const NAMESPACE_REVISION_BYTES: usize = ${formatted_decimal(v1.namespace_revision_bytes)};
/// Bytes in the fixed namespace name length field.
pub const NAMESPACE_NAME_LENGTH_BYTES: usize = ${formatted_decimal(v1.namespace_name_length_bytes)};
/// Width and missing sentinel used by the generic optional-value codec.
pub const OPTIONAL_VALUE_LENGTH_BYTES: usize = ${formatted_decimal(v1.optional_value_length_bytes ?? 4)};
pub const OPTIONAL_VALUE_MISSING: u32 = ${formatted_decimal(v1.optional_value_missing ?? 0xffff_ffff)};

/// First assigned status value reserved for errors.
pub const ERROR_STATUS_MINIMUM: u8 = ${formatted_byte(v1.error_status_minimum)};

${rust_wire_enum("Opcode", "Operations supported by protocol v1.", contract.opcodes, "UnknownOpcode")}

${rust_wire_enum("Status", "Status returned in every protocol response.", contract.statuses, "UnknownStatus")}
`
}
