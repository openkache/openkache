# Client interoperability fixtures

These public fixtures are machine-readable representatives of the draft
client and protocol contracts. They are intentionally small boundary samples;
the specifications remain the normative explanation of field meanings and
rejection behavior.

- `key_format_v1.json` contains typed-key encodings and Item ID mapping cases.
- `value_format_v1.json` contains envelope and protection vector metadata.
- `protocol_v1.json` contains frame boundary and malformed-input cases.

Fixture fields are stable within the draft revision but may change before
freeze. A fixture consumer MUST validate the declared `spec` and `revision`
before using a vector. Future generated fixtures should preserve the same
field names: `input`, `intermediate`, `output`, and `rejection_reason`.
