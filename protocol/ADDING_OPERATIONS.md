# Adding an operation

New operations are modeled once in Smithy and then consume the generic wire
and client infrastructure. The server behavior owns application meaning;
transport and generated clients own framing and codec mechanics.

## Minimal path

1. Add the Smithy input and output shapes and an operation to
   `protocol/model/openkache.smithy`.
2. Add `@operationContract` with explicit `requestFraming`,
   `responseFraming`, `successStatuses`, and `errorStatuses`.
3. Mark modeled members with `@operationField(role: "...")`. Role strings are
   open API metadata; do not add a role to a shared Rust or TypeScript enum.
4. Add `@wireCodec` only when the inferred primitive codec is insufficient.
   Codec declarations must describe the encoded value, not the operation's
   domain.
5. Regenerate the protocol and client contracts:

   ```text
   nix develop -c just generate-protocol-contract
   ```

6. Add one API-owned behavior/field binding module next to
   `server/src/operation_generic_handlers.rs`, then add its registration slice
   next to `server/src/operation_generic_registrations.rs`. Register it with
   the shared `RegistrationBuilder`. The binding should receive an
   `OperationInputView`/`OperationFieldEnvelope` and return an
   `OperationOutcome`; it must not receive a wire frame, RESP value, or client
   ABI object.
7. Add behavior tests in the private root and one live invocation to the
   managed-client smoke matrix when the operation is intended for clients.

No generic parser, dispatcher, client executor, or protocol layout branch
should mention the new operation name.

Handlers should consume borrowed field envelopes and return transport-neutral
outcomes. They must not construct a `Response`, choose a wire status byte, or
decode request framing. The generic server adapter validates generated
response requiredness, widths, and nested codecs before encoding the outcome;
client field views apply the same plan validation when they expose a response.

## Framing choice

| Shape | Request framing | Response framing | Use when |
| --- | --- | --- | --- |
| no modeled members | `empty` | `empty` or `opaque` | control/status operations |
| one already-encoded value | `opaque` | `opaque` or `empty` | pass-through or application values |
| ordered members | `ordered_fields` | `field_sequence` | optional, repeated, or mixed fields |
| generic optional result | `ordered_fields` | `field_sequence` | default for optional, repeated, or mixed fields |
| legacy compact optional result | `ordered_fields` | `adapter_owned` | only when a compatibility adapter must preserve an existing length/sentinel wire format |

`dense` is selected automatically for an all-required fixed-width ordered plan.
The legacy compact format is not a generic layout. Its compatibility adapter
owns the four-byte big-endian length/sentinel representation. A new operation
should use `field_sequence`; it must not add a route or dispatcher branch.

An unknown response framing is retained as `adapter_owned` and must be handled
by its declaring adapter. The adapter may add only body segments; generic
status validation, aggregate limits, and response ownership remain shared.

Use `requestWire` only when an operation requires exact request bytes that the
shape-selected generic layout cannot provide. Compose its fixed, packed,
byte-length, varuint, conditional, constant, and trailing-field primitives;
do not add a named route or a runtime operation branch. The generated frame
layout and semantic field plan come from that one declaration, so transport
can delimit bytes without constructing an API request type.

## Size and resource budget

Every encoded request and response is bounded by `maxValueBytes`. The generated
descriptor also records a shape-derived admission bound:

- fixed-width dense plans use their exact width, clamped to the aggregate
  ceiling;
- opaque and variable fields use the aggregate ceiling;
- sequences include one shared presence-mask prefix plus the maximum canonical
  length prefix and value bytes for every present field except the final
  present field, which consumes the remainder;
- compact optional-value tables include one four-byte prefix per modeled field;
  unknown adapter-owned tables are budgeted by their adapter.

The runtime still validates actual frame and field lengths. Generated plans are
bounded to 256 flattened fields, 64 nested codec levels, and 256 nested codec
entries. Container values are validated with bounded borrowed cursors and the
aggregate byte ceiling; a shape that intentionally exceeds the inline ceiling
needs a separate streaming/spillable protocol representation rather than a
larger generic stack table.

When changing a wire limit or adding a container shape, add tests for:

- the exact size at the limit and one byte above it;
- empty and maximum-entry containers;
- malformed, trailing, and non-canonical length encodings;
- nested codec depth and metadata mismatch;
- every generated language that exposes the operation.

Use the shared codec cursors for large lists/maps instead of collecting
temporary element vectors. Direct cursor and union/enum/UTF-8 validation also
enforces the aggregate byte ceiling, so API code cannot bypass the bound by
calling a reusable primitive outside request framing.

## Compatibility adapters

Historical namespace/item convenience types, SET option errors, and legacy
result projections belong in `protocol/compatibility_v1.ts` and the matching
server/client API adapter modules. Their exact request bytes still use the
generic `requestWire` plan. Compatibility code may translate modeled fields
into existing public types and error variants, but transport, framing, and
server dispatch must not learn that vocabulary. The protocol-v1 four-byte
big-endian length and `0xffffffff` missing sentinel are implemented once in
the compatibility-only `optional_values` codec; the v1 module re-exports it
and adds typed convenience projections.

Only an operation that preserves an existing protocol-v1 client convenience
surface should declare `compatibilityRequestProjection`. New operations omit
it, even when their modeled fields resemble namespace, item, or value fields.
The client adapter must never infer compatibility from field roles: doing so
would silently specialize an unrelated future API.

If a future API needs a different transport, add a new adapter at the
composition boundary. Do not add a built-in handler enum or a transport branch
to the generic dispatcher.
