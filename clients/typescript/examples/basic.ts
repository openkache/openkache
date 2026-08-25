/**
 * Complete Gate 0 CRUD example for a local development server.
 *
 * Run with Bun or Node after installing the published package:
 *   bun add openkache && bun run examples/basic.ts
 */

import { OpenKache_Client } from "openkache"

const endpoint = process.env.OPENKACHE_ADDRESS ?? "127.0.0.1:4433"
const client = await OpenKache_Client.connect(endpoint)

try {
  console.log("SET:", await client.set("greeting", { from: "typescript" }))

  const result = await client.get("greeting")
  console.log("GET:", result.kind === "found" ? result.value : "missing")

  console.log("DELETE:", await client.delete("greeting"))
  console.log(
    "GET after DELETE:",
    (await client.get("greeting")).kind === "missing" ? "missing" : "found",
  )
} finally {
  await client.close()
}
