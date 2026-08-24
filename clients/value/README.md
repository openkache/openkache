# OpenKache value model

`openkache-value` is the production Rust implementation of the
cross-language value model in [`SPEC.md`](SPEC.md). It deliberately has no
dependency on the client transport, cache server, value envelope, compression,
or protection profiles.

## Commands

From the public repository root, build the production target with the shared
Bazel graph:

```text
bazel build --lockfile_mode=error //clients/value:openkache_value
```

The private monorepo owns conformance tests and broader validation; no tests or
test dependencies are shipped in this public repository.

The crate exposes an owned [`Value`](https://docs.rs/openkache-value/latest/openkache_value/enum.Value.html)
algebra and its bounded structured-value payload codec:

```rust
use openkache_value::{decode, encode, Value};

let value = Value::TextString("hello".to_owned());
let bytes = encode(&value)?;
assert_eq!(decode(&bytes)?, value);
# Ok::<(), openkache_value::Error>(())
```

`decode` accepts one definite-length CBOR item and rejects trailing bytes,
indefinite-length items, unsupported tags and simple values, non-scalar or
duplicate map keys, and malformed integers. Both encoding and decoding use
explicit bounded work rather than recursive traversal. Callers that need
lower limits can use `encode_with_limits` and `decode_with_limits`.

The crate owns only logical values and their structured payload bytes. The
outer value envelope remains an independent client-core concern, and the
server continues to store the resulting bytes opaquely.
