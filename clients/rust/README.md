# OpenKache Rust client

Store, read, and delete values in an OpenKache server from Rust.

## Install

With Cargo:

```bash
cargo add openkache
cargo add tokio --features macros,rt-multi-thread
```

Or add the dependencies to `Cargo.toml`:

```toml
[dependencies]
openkache = "0.1"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

The client API is asynchronous, so the application must provide an async
runtime. The default transport uses Tokio; if the application already uses
Tokio, only `openkache` needs to be added. Rust 1.85 or newer and a native C
linker are required to build the default TLS backend.

## Quick start

The example below assumes a local OpenKache server at `127.0.0.1:4433`.

```rust
use openkache::{Client, Value};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::connect("127.0.0.1:4433").await?;

    println!("SET: {:?}", client.set("greeting", Value::text("hello")).await?);
    println!("GET: {:?}", client.get("greeting").await?);
    println!("DELETE: {:?}", client.delete("greeting").await?);
    println!("GET after DELETE: {:?}", client.get("greeting").await?);

    client.close().await?;
    Ok(())
}
```

The local development TLS profile does not verify the server certificate. Use
this example only with a local development server.

## Reference

### Client

| API | Description |
| --- | --- |
| `Client::connect(endpoint)` | Connect to a `host:port` endpoint and return an async client. IPv6 endpoints use `[host]:port`. |
| `client.get(key)` | Return `GetResult::Found(value)` or `GetResult::Missing`. |
| `client.set(key, value)` | Store a value and return `SetOutcome::Created` or `SetOutcome::Replaced`. |
| `client.delete(key)` | Delete a key and return `DeleteOutcome::Deleted` or `DeleteOutcome::NotFound`. |
| `client.close()` | Close the connection and wait for admitted operations to settle. Repeated calls are safe. |

Keys can be text, bytes, or signed 64-bit integers. Use `TypedKey::text`,
`TypedKey::bytes`, or `TypedKey::integer` when you want to choose the key type
explicitly.

### Results and errors

- `GetResult<T>` is `Missing` or `Found(T)`. A stored `Value::Null` or
  `Value::Undefined` is still `Found`.
- `SetOutcome` is `Created` or `Replaced`.
- `DeleteOutcome` is `Deleted` or `NotFound`.
- `Error::UnknownMutation` means a `set` or `delete` may have reached the
  server without a confirmed result. Its `Mutation` value is `Set` or `Delete`;
  do not replay the operation automatically.
- `Error::Core` reports connection, protocol, key, value, or server failures.
- `Error::UnsupportedSetOutcome` reports a server result outside this client's
  unconditional-write API.
- `Result<T>` is the result alias returned by client methods.

### Values

`Value` is the value type accepted by `set` and returned by `get`:

```text
Undefined | Null | Boolean | Integer | Float | TextString | Bytes | Array | Map
```

Common constructors and conversions:

| API | Description |
| --- | --- |
| `Value::integer(value)` | Create an exact integer. |
| `Value::float16(bits)` / `float32(bits)` / `float64(bits)` | Create a float from exact IEEE-754 bits. |
| `Value::text(value)` | Create UTF-8 text. |
| `Value::bytes(value)` | Create bytes. |
| `Value::array(values)` | Create an ordered array. |
| `Value::map(entries)` | Create an ordered map with scalar, unique keys. |
| `Value::to_cbor()` / `Value::from_cbor(bytes)` | Encode or decode one complete structured value. |

For values that need explicit precision or large integers:

- `Integer::zero`, `Integer::from_i128`, `Integer::from_u128`, and
  `Integer::parse_decimal`
- `Sign::Positive` and `Sign::Negative` describe an `Integer` magnitude.
- `Float::new` with `FloatWidth::Bits16`, `Bits32`, or `Bits64`
- `ValueLimits` to bound value size, depth, item count, and integer size
- `ValueError` for value conversion and encoding errors

### Public key helpers

- `TypedKey::text(value)` creates a text key.
- `TypedKey::bytes(value)` creates a byte key.
- `TypedKey::integer(value)` creates an integer key.
- `TypedKey::canonical_bytes()` returns the canonical key representation.
- `KeyError` describes invalid or out-of-range keys.

## More information

- [OpenKache on crates.io](https://crates.io/crates/openkache)
- [Rust API reference](https://docs.rs/openkache)
- [OpenKache repository](https://github.com/openkache/openkache)
