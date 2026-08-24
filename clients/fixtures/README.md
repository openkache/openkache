# Client interoperability fixtures

These public JSON files are the canonical machine-readable representation of
the OpenKache maintained client contract frozen at `v1-gate0`. They are
contract data, not tests or private development infrastructure. The normative
explanation of field meanings and rejection behavior remains in the linked
public specifications.

- `schema.json` validates the common fixture structure.
- `client_contract_v1.json` defines the five operations, development TLS trust,
  lookup result tags, mutation outcomes, unknown mutations, and unsupported
  features.
- `key_format_v1.json` defines `Integer`, `Text`, and `Bytes` keys, canonical
  bytes, the shared `NamespaceHash` profile, and key rejection.
- `structured_value_cbor_v1.json` covers every model value kind and CBOR
  rejection rule.
- `value_format_v1.json` defines the fixed `01 10` Gate 0 envelope and
  rejects raw, JSON, compressed, protected, and caller-owned-v0 selectors.
- `protocol_v1.json` carries the stable GET/SET/DELETE frame boundaries owned
  by the wire protocol.

Every file declares `spec_revision = "v1-gate0"`. Consumers MUST validate
both `spec` and `spec_revision`, and MUST treat explicit type fields as
normative; a JSON string or number never supplies an implicit key/value type.
Every vector contains `kind`, `name`, `input`, `intermediate`, `output`, and
`error`. Positive, negative, and boundary vectors use the same schema.

The revision is a compatibility identifier, not an edit date. Changing an
encoding, assignment, validation result, or fixture shape requires a new
contract revision and coordinated updates to the public specifications.
Cross-language tests and private validation consume these vectors from the
private monorepo; no test files are added to this public repository.
