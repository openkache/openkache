# OpenKache Rust client

OpenKache is a super-fast open-source SSD cache server. Use this async Rust
client to store, read, and delete values in a few lines.

[crates.io package](https://crates.io/crates/openkache) ·
[docs.rs API reference](https://docs.rs/openkache/latest/openkache/) ·
[GitHub source](https://github.com/openkache/openkache/tree/main/clients/rust)

## Install

Rust 1.85 or newer and a native C linker are required.

```bash
# OpenKache client
cargo add openkache

# Tokio, for the standalone example's #[tokio::main]
cargo add tokio --features macros,rt-multi-thread
```

The client uses Tokio internally, so an active Tokio runtime is required.
`openkache` already brings Tokio into the dependency graph; add Tokio directly
only when your application needs the `#[tokio::main]` macro. If the application
already uses Tokio, skip the second command. Tokio is the supported runtime for
this client.

## Quick start

The example below assumes a local OpenKache server at `127.0.0.1:4433`.

```rust
use openkache::Client;

#[tokio::main]
async fn main() -> openkache::Result<()> {
    let client = Client::connect("127.0.0.1:4433").await?;

    client.set("greeting", "hello").await?;
    println!("{:?}", client.get("greeting").await?);
    client.delete("greeting").await?;
    client.close().await?;
    Ok(())
}
```

The local development TLS profile does not verify the server certificate. Use
this example only with a local development server.

`Value` is the Rust type used for structured values. Writes accept common Rust
values directly and convert them to `Value`; construct an explicit variant
when float width, raw bits, or model map keys matter.

## Reference

### `Client::connect(endpoint)`

Opens a connection and returns a `Client`.

- **Input:** a `host:port` endpoint. IPv6 endpoints use `[host]:port`.
- **Returns:** an async `Result<Client>`.
- **Errors:** `Error::Core` when the endpoint, TLS handshake, or protocol
  setup fails.

```rust
let client = Client::connect("127.0.0.1:4433").await?;
```

### `client.get(key)`

Reads one `Value`.

- **Input:** anything that converts into `TypedKey`: text, bytes, or a signed
  64-bit integer.
- **Returns:** `Ok(GetResult::Found(value))` when the key exists, or
  `Ok(GetResult::Missing)` when it does not. A stored `Value::Null` or
  `Value::Undefined` is still `Found`.
- **Errors:** `Error::Core` for connection, protocol, key, or value failures.

```rust
match client.get("greeting").await? {
    GetResult::Found(value) => println!("{value:?}"),
    GetResult::Missing => println!("missing"),
}
```

### `client.set(key, value)`

Stores one value with an unconditional write.

- **Input:** a `TypedKey`-convertible key and any value that implements
  `Into<Value>`. Common strings, byte slices, booleans, integers, and floats
  are accepted directly.
- **Returns:** `Ok(SetOutcome::Created)` for a new key or
  `Ok(SetOutcome::Replaced)` for an existing key.
- **Errors:** `Error::UnknownMutation` when admission happened but the result
  was not confirmed; do not replay that mutation. Other failures are returned
  as `Error::Core`.

```rust
let outcome = client.set("greeting", "hello").await?;
```

### `client.delete(key)`

Deletes one key. Repeating the operation is safe.

- **Input:** a `TypedKey`-convertible key.
- **Returns:** `Ok(true)` when a value was removed, or `Ok(false)` when no
  value existed.
- **Errors:** `Error::UnknownMutation` when the result of an admitted delete
  was not confirmed.

```rust
let removed = client.delete("greeting").await?;
if removed {
    println!("deleted");
}
```

### `client.close()`

Closes the connection after admitted operations settle. Repeated calls are
safe.

- **Returns:** `Result<()>`.

```rust
client.close().await?;
```

### Keys

Use `TypedKey` constructors when you want to make the key type explicit:

- `TypedKey::text(value)` creates a UTF-8 text key.
- `TypedKey::bytes(value)` creates an exact byte key.
- `TypedKey::integer(value)` creates a signed 64-bit integer key.
- `TypedKey::canonical_bytes()` returns the encoded key.

The client also accepts common Rust conversions directly:

```rust
client.get("text-key").await?;
client.get(b"bytes-key").await?;
client.get(42_i64).await?;
```

`KeyError` describes invalid or out-of-range keys.

### Values

`Value` is the structured value type returned by `get` and accepted by `set`:

```text
Undefined | Null | Boolean | Integer | Float | TextString | Bytes | Array | Map
```

Construct values with:

- `Value::integer(value)` for an exact integer.
- `Value::float16(bits)`, `Value::float32(bits)`, or `Value::float64(bits)`
  for exact IEEE-754 bits.
- `Value::text(value)` for UTF-8 text.
- `Value::bytes(value)` for exact bytes.
- `Value::array(values)` for an ordered array.
- `Value::map(entries)` for an ordered map with scalar, unique keys.
- `Value::to_cbor()` and `Value::from_cbor(bytes)` to encode or decode one
  complete structured value.

For common writes, the client also accepts `&str`, `String`, `&[u8]`,
`Vec<u8>`, booleans, all signed and unsigned integer types, `f32`, and `f64`:

```rust
client.set("name", "Ada").await?;
client.set("count", 42_u64).await?;
client.set("payload", b"bytes").await?;
```

```rust
let value = Value::map(vec![
    (Value::text("count"), Value::integer(1_i128)),
])?;
client.set("stats", value).await?;
```

For exact integer and float construction, use `Integer`, `Sign`,
`Float`, and `FloatWidth`. `ValueLimits` bounds bytes, depth, item count, and
integer magnitude. `ValueError` reports value validation and encoding errors;
use `ValueError::kind()` for its stable category.

`Result<T>` is the result alias returned by client methods.

### Results and errors

- `GetResult<T>` is `Missing` or `Found(T)`.
- `SetOutcome` is `Created` or `Replaced`.
- `delete` returns `true` when an item existed and `false` otherwise.
- `Error::UnknownMutation` means a `set` or `delete` may have reached the
  server without a confirmed result. Its `Mutation` identifies the operation;
  do not replay it automatically.
- `Error::Core` reports connection, protocol, key, value, or server failures.
- `Error::UnsupportedSetOutcome` reports a server result outside the
  unconditional-write API.

## More information

- [OpenKache on crates.io](https://crates.io/crates/openkache)
- [Rust API reference](https://docs.rs/openkache/latest/openkache/)
- [OpenKache repository](https://github.com/openkache/openkache)
