# Client interoperability fixtures

These public fixtures are machine-readable representatives of the draft
client and protocol contracts. They are intentionally small boundary samples;
the specifications remain the normative explanation of field meanings and
rejection behavior.

- `schema.json` validates the common fixture structure.
- `key_format_v1.json` contains typed-key encodings and Item ID mapping cases.
- `value_format_v1.json` contains envelope and protection vector metadata.
- `protocol_v1.json` contains frame boundary and malformed-input cases.

Fixture fields are stable only within the declared draft revision and may
change before freeze. A consumer MUST validate `spec` and `spec_revision`.
Every vector declares `kind`, `input`, `intermediate`, `output`, and `error`.
Positive, negative, and boundary cases use the same schema.

`spec_revision` identifies a byte contract, not an edit date. Any change to an
encoding, assignment, validation result, or fixture schema requires a new
revision in the specification and every fixture that implements it.

Before v1 freezes, generated fixtures MUST cover:

- every opcode, status, selector, and flag assignment;
- every `vu128`, Item ID length, signed `i64`, and value-size boundary;
- cross-lane linearization and every mutation effect category;
- every protected-value profile, immutable key-ID rejection, and key
  substitution failure; and
- Zstandard content-size, window, dictionary, multi-frame, truncation, and
  trailing-byte rejection.

Every normative assignment needs at least one positive or boundary vector, and
every rejection rule needs a negative vector. The freeze gate MUST validate the
JSON Schema and coverage matrix. At least two independent implementations must
reproduce the positive vectors and reject the negative vectors.
