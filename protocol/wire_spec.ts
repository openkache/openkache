/** Transport-only documentation helpers. */

import type { Wire_Contract } from "./wire"

/** Marker for the checked-in transport snapshot in `SPEC.md`. */
export const PROTOCOL_SPEC_CONTRACT_SNAPSHOT_START =
  "<!-- openkache:generated-protocol-contract-snapshot:start -->"
export const PROTOCOL_SPEC_CONTRACT_SNAPSHOT_END =
  "<!-- openkache:generated-protocol-contract-snapshot:end -->"

/** Renders the stable transport constants snapshot. */
export function render_protocol_spec_contract_snapshot(
  contract: Wire_Contract,
): string {
  const v1 = contract.v1
  return `| Transport constant | Value |
|---|---|
| ALPN | \`${v1.alpn}\` |
| Maximum payload bytes | \`${contract.max_payload_bytes}\` |
| Request code bytes | \`${v1.request_code_bytes}\` |
| Response code bytes | \`${v1.response_code_bytes}\` |
| Minimum varuint bytes | \`${v1.min_varuint_bytes}\` |
| Maximum varuint bytes | \`${v1.max_varuint_bytes}\` |`
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
