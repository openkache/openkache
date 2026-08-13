# Adding an API operation

Protocol v1 is still an unpublished, evolving draft profile. The transport
crate intentionally does not generate operation codecs, server adapters, or
client methods. Every API uses the same explicit implementation boundary.

## Minimal path

1. Document the operation and its shapes in the protocol model if it is part
   of the shared draft.
2. Reserve an opcode only when the operation needs a wire-visible assignment.
   The transport generator emits the opaque assignment; it does not infer the
   operation body.
3. Add an API-owned module that implements:

   - request serialization and deserialization;
   - response serialization and deserialization;
   - semantic result/status projection;
   - request frame layout, when the operation is not length-delimited;
   - resource preparation, authorization, and handler behavior;
   - client-facing convenience methods, if the API has a client package.

4. Register the module with the generic server/client transport boundary.
   Registration supplies an opcode, frame delimiter, request decoder, handler,
   result projector, and retry/commit policy. The transport sees only those
   callbacks and opaque bytes.
5. Use the protocol utilities where they match the contract:

   - `decode_varuint` / `encode_varuint`;
   - `OpaqueRequestFrame` and `ResponseParts`;
   - `FieldSequence` and field-group helpers;
   - `DenseFields` for required fixed-width tuples;
   - `OptionalValueCodec` when the API explicitly specifies a fixed-prefix
     optional-value table;
   - borrowed cursors and segmented response values for large payloads.

There is no generated operation plan to update and no built-in handler table to
extend. Adding an API should be a local module plus one registration entry.

## Choosing a layout

| Contract shape | Suggested primitive |
| --- | --- |
| no body | empty request/response |
| one already-encoded value | opaque body |
| ordered optional or variable fields | `FieldSequence` |
| required fixed-width tuple | `DenseFields` |
| explicit fixed-prefix optional table | `OptionalValueCodec` |
| nested/repeated values | API codec over shared container cursors |

`OptionalValueCodec` is not a built-in operation family. The API supplies both
the prefix width and the missing sentinel, so present-empty and missing remain
distinct without embedding any domain-specific number in generic code.

## Boundary rules

The generic server/client layers must not branch on operation names, namespace
or item roles, domain enums, route families, status meanings, or client ABI
types. They delimit frames, retain ownership, dispatch a registered decoder,
and write/read opaque response parts.

An API handler must not parse QUIC/RESP details or construct a transport frame.
It returns an API-owned semantic result which its registration projector maps
to the selected wire bytes.

For large values, keep response segments borrowed or ownership-preserving
until the transport write. Copy only at a language binding boundary that
promises an owned value.

## Review checklist

- Does the new API compile without editing a generic codec or dispatcher?
- Are empty, missing, present-empty, malformed, trailing, and maximum values
  covered?
- Are nested lists/maps/unions validated without collecting unnecessary
  temporary vectors?
- Is the wire contract documented before the implementation?
- Can the API be removed by deleting its module and registration entry?
