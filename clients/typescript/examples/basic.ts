/**
 * Complete CRUD example for a local development server.
 *
 * Run with Bun or Node after installing the published package:
 *   bun add openkache && bun run examples/basic.ts
 */

import { OpenKacheClient } from "openkache"

const endpoint = process.env.OPENKACHE_ADDRESS ?? "127.0.0.1:4433"
const client = await OpenKacheClient.connect(endpoint)

try {
  console.log("SET:", await client.set("greeting", { from: "typescript" }))

  const result = await client.get("greeting")
  console.log("GET:", result)

  console.log("DELETE:", await client.delete("greeting"))
  const after_delete = await client.get("greeting")
  console.log(
    "GET after DELETE:",
    after_delete === undefined ? "missing" : after_delete,
  )
} finally {
  await client.close()
}
