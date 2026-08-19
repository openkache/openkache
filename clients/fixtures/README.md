# Client interoperability fixtures

These public fixtures are machine-readable representatives of the draft
client and protocol contracts. They are intentionally small boundary samples;
the specifications remain the normative explanation of field meanings and
rejection behavior.

- `key_format_v1.json` contains typed-key encodings and Item ID mapping cases.
- `value_format_v1.json` contains envelope and protection vector metadata.
- `protocol_v1.json` contains frame boundary and malformed-input cases.

Fixture fields are stable only within the declared draft revision and may
change before freeze. A consumer MUST validate `spec` and `spec_revision`.
Every vector declares `kind`, `input`, `intermediate`, `output`, and `error`.
Positive, negative, and boundary cases use the same schema.
