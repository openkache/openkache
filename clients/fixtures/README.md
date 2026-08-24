# Client interoperability fixtures

> **Status:** Frozen Gate 0 (`v1-gate0`, 2026-08-24).

These public JSON files are the canonical machine-readable representation of
the OpenKache maintained client and protocol contracts frozen at `v1-gate0`.
They are contract data, not tests or private development infrastructure; the
specifications remain the normative explanation of field meanings and
rejection behavior.

- `schema.json` validates the common fixture structure.
- `client_contract_v1.json` defines the exact five-operation Gate 0 facade,
  development TLS trust, lookup/mutation outcomes, unknown mutations, and
  unsupported features.
- `key_format_v1.json` contains typed-key encodings, all mapping-profile
  boundaries, and Item ID derivation cases.
- `structured_value_cbor_v1.json` covers every model value kind and CBOR
  rejection rule.
- `value_format_v1.json` contains the complete envelope, compression,
  protection, caller-owned-v0, and Gate 0 selector vectors.
- `protocol_v1.json` contains frame boundary and malformed-input cases.

Fixture fields are stable within the declared `v1-gate0` revision. A consumer
MUST validate `spec` and `spec_revision`.
Every vector declares `kind`, `input`, `intermediate`, `output`, and `error`.
Positive, negative, and boundary cases use the same schema.

`spec_revision` identifies a byte contract, not an edit date. Any change to an
encoding, assignment, validation result, or fixture schema requires a new
revision in the specification and every fixture that implements it.

The frozen fixture set covers:

- every opcode, status, selector, and flag assignment;
- every `vu128`, Item ID length, signed `i64`, and value-size boundary;
- cross-lane linearization and every mutation effect category;
- every protected-value profile, immutable key-ID rejection, and key
  substitution failure;
- Zstandard content-size, window, dictionary, multi-frame, truncation, and
  trailing-byte rejection.

Every normative assignment has at least one positive or boundary vector, and
every rejection rule has a negative vector. Consumers MUST validate the JSON
Schema and coverage matrix. Cross-language tests and private validation consume
these vectors from the private monorepo; no test files are added to this public
repository.
