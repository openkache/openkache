# Adding an API operation

Protocol v1 is still an unpublished, evolving draft profile. A stable operation
first receives its registry decision in `SPEC.md`; only then does its
transitional Smithy model metadata describe the compact wire contract. Each API
implements its semantics behind operation-neutral protocol and server
boundaries.

## Minimal path

1. Decide the operation's lifecycle first. A stable-v1 operation MUST be
   assigned an opcode, allowed statuses, and a complete frame layout in
   [`SPEC.md`](SPEC.md), which is the normative registry. Do not treat a
   Smithy enum member, generated status, or draft operation name as an
   assignment. Experimental operations require an exact draft revision and
   remain outside stable-v1 conformance; namespace-management operations are
   `outOfBand` until a separate wire assignment is approved.
2. Add the operation, request and response shapes, field codecs, and (only
   after the registry decision) its transitional metadata to the Smithy model.
3. For every non-empty request, declare `requestWire` with the neutral fixed,
   packed, conditional, length, constant, and trailing-field primitives.
   Empty requests are the only requests that omit a wire plan.
4. Regenerate the contract. Generation emits:

   - numeric request and response field modules;
   - operation framing, status, and codec metadata;
   - the shared request frame layout used by the encoder and projector.

5. Add a typed API adapter. It maps domain values to canonical numeric fields
   and uses the shared request encoder/projector instead of duplicating frame
   parsing or serialization.
6. Add the API-owned server binding and handler registration. The binding
   performs semantic decoding, capability and resource preparation,
   authorization, behavior, and response projection. The generic server owns
   admission, scheduling, lifecycle, dispatch, and transport writes.
7. Add a client-facing method when the API has a client package. Keep retry
   policy and semantic result mapping in the API adapter, outside the server
   lifecycle.

## Boundary rules

Adding an API must not add an operation-name, API-family, field-role, route, or
status-meaning branch to generic protocol, transport, scheduler, dispatcher, or
lifecycle code. Generic code consumes generated numeric metadata and registered
callbacks.

An API handler must not parse transport frames or depend on client types. It
returns an API-owned semantic result which its response projector maps to the
declared wire contract.

For large byte fields, project borrowed ranges from the owned request frame and
transfer ownership into storage when needed. Keep response segments borrowed
or ownership-preserving until the transport write. Copy only at a boundary
whose contract requires an owned value.

## Review checklist

- Does the operation declare an explicit compact request plan?
- Does it compile without editing a generic codec, scheduler, dispatcher, or
  lifecycle?
- Are empty, missing, present-empty, malformed, trailing, and maximum values
  covered?
- Are nested lists/maps/unions validated without collecting unnecessary
  temporary vectors?
- Does the typed adapter use generated numeric fields and the shared request
  encoder/projector?
- Are large byte fields borrowed or ownership-transferred rather than copied?
- Can the API be removed by deleting its module and registration entry?
