# Adding an API operation

Protocol v1 is still an unpublished, evolving draft profile. The transport
crate intentionally does not generate operation codecs, server adapters, or
client methods. Every API uses the same explicit implementation boundary.

## Minimal path

1. Write the wire contract first: choose the request/response code values,
   envelope layout, field order, codecs, validation, and semantic outcomes in
   the API's own module or contract document.
2. Add an API-owned module that implements:

   - request serialization and deserialization;
   - response serialization and deserialization;
   - semantic result/status projection;
   - request frame layout, when the operation is not length-delimited;
   - resource preparation, authorization, and handler behavior;
   - client-facing convenience methods, if the API has a client package.

3. Register the module with the generic server/client transport boundary.
   Registration supplies opaque request/response code values, a frame
   delimiter, request decoder, handler, result projector, and retry/commit
   policy. The transport sees only those callbacks and opaque bytes.
4. Use the protocol utilities where they match the contract:

   - `decode_varuint` / `encode_varuint`;
   - `OpaqueRequestFrame` and `ResponseParts`;
   - `FieldSequence` and field-group helpers;
   - `DenseFields` for required fixed-width tuples;
   - `OptionalValueCodec` when the API explicitly specifies a fixed-prefix
     optional-value table;
   - segmented response values for large payloads.

There is no generated operation plan or built-in handler table to extend.
Adding an API should be a local contract/codec module plus one registration
entry. Adding it must not require editing this generic crate.

## Choosing a layout

| Contract shape | Suggested primitive |
| --- | --- |
| no body | empty request/response |
| one already-encoded value | opaque body |
| ordered optional or variable fields | `FieldSequence` |
| required fixed-width tuple | `DenseFields` |
| explicit fixed-prefix optional table | `OptionalValueCodec` |
| nested/repeated values | API-owned codec over its documented container format |

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
