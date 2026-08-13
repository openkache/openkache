/** Transport-only documentation helpers. */

import type { Wire_Contract } from "./wire"

/** Marker for the checked-in transport snapshot in `SPEC.md`. */
export const PROTOCOL_SPEC_CONTRACT_SNAPSHOT_START =
  "<!-- openkache:generated-protocol-contract-snapshot:start -->"
export const PROTOCOL_SPEC_CONTRACT_SNAPSHOT_END =
  "<!-- openkache:generated-protocol-contract-snapshot:end -->"

function wire_name(identifier: string): string {
  return identifier
    .replace(/([a-z0-9])([A-Z])/g, "$1_$2")
    .replace(/([A-Z]+)([A-Z][a-z])/g, "$1_$2")
    .toLowerCase()
}

/** Renders the stable transport identifier and limit snapshot. */
export function render_protocol_spec_contract_snapshot(
  contract: Wire_Contract,
): string {
  const opcode_rows = contract.opcodes
    .map(
      (entry) =>
        `| \`${entry.value.toString(16).padStart(2, "0").toUpperCase()}\` | \`${wire_name(entry.name).toUpperCase()}\` |`,
    )
    .join("\n")
  const status_rows = contract.statuses
    .map(
      (entry) =>
        `| \`${entry.value.toString(16).padStart(2, "0").toUpperCase()}\` | \`${wire_name(entry.name).toUpperCase()}\` |`,
    )
    .join("\n")
  const v1 = contract.v1
  return `| Transport constant | Value |
|---|---|
| ALPN | \`${v1.alpn}\` |
| Item ID bytes | \`${contract.item_id_bytes}\` |
| Maximum payload bytes | \`${contract.max_value_bytes}\` |
| Opcode bytes | \`${v1.opcode_bytes}\` |
| Status bytes | \`${v1.status_bytes}\` |
| Request fixed bytes | \`${v1.request_fixed_bytes}\` |
| Response fixed bytes | \`${v1.response_fixed_bytes}\` |
| Minimum varuint bytes | \`${v1.min_varuint_bytes}\` |
| Maximum varuint bytes | \`${v1.max_varuint_bytes}\` |

### Opcodes

| Value | Name |
|---|---|
${opcode_rows}

### Statuses

| Value | Name |
|---|---|
${status_rows}`
}

export function protocol_spec_contract_snapshot_issues(
  spec: string,
  contract: Wire_Contract,
): readonly string[] {
  const start = spec.indexOf(PROTOCOL_SPEC_CONTRACT_SNAPSHOT_START)
  const end = spec.indexOf(PROTOCOL_SPEC_CONTRACT_SNAPSHOT_END)
  if (start < 0 || end < start) {
    return ["protocol/SPEC.md (transport snapshot markers missing)"]
  }
  const actual = spec
    .slice(start + PROTOCOL_SPEC_CONTRACT_SNAPSHOT_START.length, end)
    .trim()
  const expected = render_protocol_spec_contract_snapshot(contract).trim()
  return actual === expected ? [] : ["protocol/SPEC.md (transport snapshot stale)"]
}
