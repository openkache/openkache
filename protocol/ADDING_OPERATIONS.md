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

6. Add one API-owned binding in `server/src/operation_generic_bindings.rs`
   (or a separate API module), then register it with the shared
   `RegistrationBuilder`. The binding should receive an
   `OperationInputView`/`OperationFieldEnvelope` and return an
   `OperationOutcome`; it must not receive a wire frame, RESP value, or client
   ABI object.
7. Add behavior tests in the private root and one live invocation to the
   managed-client smoke matrix when the operation is intended for clients.

No generic parser, dispatcher, client executor, or protocol layout branch
should mention the new operation name.

Handlers should consume borrowed field envelopes and return transport-neutral
outcomes. They must not construct a `Response`, choose a wire status byte, or
decode a compatibility route. The generic server adapter validates generated
response requiredness, widths, and nested codecs before encoding the outcome;
client field views apply the same plan validation when they expose a response.

## Framing choice

| Shape | Request framing | Response framing | Use when |
| --- | --- | --- | --- |
| no modeled members | `empty` | `empty` or `opaque` | control/status operations |
| one already-encoded value | `opaque` | `opaque` or `empty` | pass-through or application values |
| ordered members | `ordered_fields` | `field_sequence` | optional, repeated, or mixed fields |
| historical v1 result | `ordered_fields` | `optional_values` | only when preserving the v1 byte contract |

`dense` is selected automatically for an all-required fixed-width ordered plan.
It is a layout optimization, not an operation family. A new operation must not
select `optional_values` merely because it returns multiple values; use the
generic sequence unless compatibility requires the historical table.

## Size and resource budget

Every encoded request and response is bounded by `maxValueBytes`. The generated
descriptor also records a shape-derived admission bound:

- fixed-width dense plans use their exact width, clamped to the aggregate
  ceiling;
- opaque and variable fields use the aggregate ceiling;
- sequences include one shared presence-mask prefix plus the maximum canonical
  length prefix and value bytes for every present field except the final
  present field, which consumes the remainder;
- optional-value tables include their fixed four-byte prefixes.

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

Protocol-v1 compact routes, namespace/item identity, SET flags, and legacy
result projections belong in `protocol/compatibility_v1.ts` and the matching
server/client adapter modules. An adapter may validate its own namespaced
operation-contract extensions, but generic extraction must preserve unknown
extensions opaquely and must not learn that adapter's vocabulary.

If a future API needs a different transport, add a new adapter at the
composition boundary. Do not add a built-in handler enum or a transport branch
to the generic dispatcher.
