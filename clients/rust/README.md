# OpenKache Rust client

OpenKache is a high-performance cache server designed from the ground up for
modern SSDs. Use this async Rust
client to store, read, and delete values in a few lines.

[crates.io package](https://crates.io/crates/openkache) ·
[docs.rs API reference](https://docs.rs/openkache/latest/openkache/) ·
[GitHub source](https://github.com/openkache/openkache/tree/main/clients/rust)

## Install

Rust 1.85 or newer and a native C linker are required.

```bash
# OpenKache client
cargo add openkache
```

The default `quic-quinn` client uses Tokio internally, so an active Tokio
runtime is required for `Client`. The example below uses `#[tokio::main]`; use
the runtime setup already used by your application.

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

The Rust client uses OpenKache's structured value format.

## Serde Rust values

`Client::set_serde` and `Client::get_serde` use Serde while retaining the
same lossless structured-value format as `set` and `get`. Add Serde to the
application when defining types for these helpers:

```bash
cargo add serde --features derive
```

```rust
use openkache::Client;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
struct User {
    id: u64,
    name: String,
}

async fn example() -> openkache::Result<()> {
    let client = Client::connect("127.0.0.1:4433").await?;
    client
        .set_serde("user:1", &User { id: 7, name: "Ada".into() })
        .await?;
    let user: User = client.get_serde("user:1").await?.unwrap();
    assert_eq!(user.id, 7);
    assert_eq!(user.name, "Ada");
    client.close().await?;
    Ok(())
}
```

`get_serde` returns `Result<Option<T>>`; in this example, `T` is `User`, so a
hit is `Some(User)`. The final `unwrap` handles the expected hit in this
small example; production code should decide how a missing key is handled.

`None` remains distinct from a stored `Value::Null` for `get`; a stored `null`
decodes as `Some(None)` for `get_serde::<Option<T>>`. Integers are
range-checked, floating-point values retain their IEEE-754 bits, and map keys
must be scalar and unique. Serde serialization happens before write admission,
so `Error::SerdeSerialize` cannot produce an unknown mutation. A value that
does not match the requested type returns `Error::SerdeDeserialize`.

`set_serde` stores the structured model as `StructuredValue-CBOR-v1`, not as
an opaque Rust-specific payload. Python and JavaScript clients can therefore
read a value written by `set_serde` as a structured object, array, scalar, or
null. Keep the schema within the cross-language value model; opaque bytes,
unsupported map keys, and serializer-specific representations remain
application-owned.

### Structured codecs with `get_with` and `set_with`

For a non-Serde structured serializer, implement `ValueCodec<T>` or construct
one with `FunctionCodec::new(encode, decode)`. This complete example maps a
`Point` struct to a structured `Value` map:

`FunctionCodec` is a small adapter around two application functions:
`encode(&T) -> Result<Value, EncodeError>` runs before a write is admitted, and
`decode(Value) -> Result<T, DecodeError>` runs after a value is retrieved. The
client does not inspect or invoke the application's serializer; it only stores
the resulting structured `Value`. An encode failure is returned as
`Error::CodecEncode`, and a decode failure as `Error::CodecDecode`.

```rust
use openkache::{Client, FunctionCodec, Value};

#[derive(Debug, PartialEq)]
struct Point {
    x: i64,
    y: i64,
}

fn encode_point(point: &Point) -> Result<Value, &'static str> {
    Value::map(vec![
        (Value::text("x"), Value::integer(point.x)),
        (Value::text("y"), Value::integer(point.y)),
    ])
    .map_err(|_| "point fields must be unique scalar keys")
}

fn decode_point(value: Value) -> Result<Point, &'static str> {
    let entries = value.as_map().ok_or("expected a point object")?;
    let mut x = None;
    let mut y = None;
    for (key, value) in entries {
        let key = match key {
            Value::TextString(key) => key,
            _ => return Err("point fields must be text keys"),
        };
        let value = match value {
            Value::Integer(value) => value.as_i128().ok_or("point must fit in i128")?,
            _ => return Err("point fields must be integers"),
        };
        match key.as_str() {
            "x" => x = Some(value),
            "y" => y = Some(value),
            _ => return Err("unknown point field"),
        }
    }
    Ok(Point {
        x: x.ok_or("missing x")?,
        y: y.ok_or("missing y")?,
    })
}

async fn example() -> openkache::Result<()> {
    let client = Client::connect("127.0.0.1:4433").await?;
    let point = Point { x: 3, y: 4 };
    let point_codec = FunctionCodec::new(encode_point, decode_point);
    client.set_with("point:1", &point, &point_codec).await?;
    assert_eq!(
        client.get_with("point:1", &point_codec).await?.unwrap(),
        point,
    );
    client.close().await?;
    Ok(())
}
```

## Custom binary payloads

Use byte payloads when the application owns the serialized format. This example
uses bincode 2; pin the serializer major version and configuration because the
bytes are an application schema contract:

```bash
cargo add bincode@2 --features serde
cargo add serde --features derive
```

The client stores and returns the bytes without inspecting them; other
language clients receive a byte value and need the same application format to
decode it:

```rust
use bincode::config;
use openkache::{Client, Value};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, PartialEq, Serialize)]
struct Session {
    user_id: u64,
    flags: u8,
}

fn invalid_data(message: &'static str) -> Box<dyn std::error::Error> {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message).into()
}

async fn example() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::connect("127.0.0.1:4433").await?;
    let session = Session {
        user_id: 7,
        flags: 0b101,
    };
    let payload = bincode::serde::encode_to_vec(&session, config::standard())?;
    client
        .set("session:1", Value::bytes(payload))
        .await?;

    let stored = client.get("session:1").await?;
    let bytes = match &stored {
        Some(Value::Bytes(bytes)) => bytes,
        Some(_) => return Err(invalid_data("expected opaque bytes")),
        None => return Err(invalid_data("session is missing")),
    };
    let (decoded, _bytes_read): (Session, usize) =
        bincode::serde::decode_from_slice(bytes, config::standard())?;
    assert_eq!(decoded, session);
    client.close().await?;
    Ok(())
}
```

Application-owned payloads normally use ordinary `get`/`set`. A
`ValueCodec` can wrap an opaque serializer only when it maps the encoded bytes
to and from `Value::Bytes`; the client still does not inspect that format.

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
- **Returns:** `Ok(Some(value))` when the key exists, or `Ok(None)` when it
  does not. A stored `Value::Null` or `Value::Undefined` is still `Some`.
- **Errors:** `Error::Core` for connection, protocol, key, or value failures.

```rust
match client.get("greeting").await? {
    Some(value) => println!("{value:?}"),
    None => println!("missing"),
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

### `client.get_serde<T>(key)`

Reads one value and decodes it into `T: serde::de::DeserializeOwned`.

- **Returns:** `Ok(Some(value))` for a matching stored value, or `Ok(None)`
  when the key does not exist.
- **Errors:** `Error::SerdeDeserialize` for a type mismatch, overflow, or
  unsupported stored value; transport and protocol failures use
  `Error::Core`.

### `client.set_serde(key, value)`

Serializes `value: impl serde::Serialize` and stores it with an unconditional
write. Serde serialization completes before network admission.

- **Returns:** the same `SetOutcome` as `client.set`.
- **Errors:** `Error::SerdeSerialize` for values that cannot be represented
  by the structured model, or the transport and mutation errors from
  `client.set`.

### `client.get_with` and `client.set_with`

Use an application-provided `ValueCodec<T>` for a non-Serde structured format.
`FunctionCodec::new(encode, decode)` is the convenient form when two functions
are enough; implement `ValueCodec<T>` directly when the codec needs state or
more control. The codec operates on `Value`, so it does not alter wire
admission or mutation semantics. Encoding happens before admission, so a
codec error cannot produce `Error::UnknownMutation`; decoding happens after
retrieval and leaves `None` unchanged. The value type is normally inferred from
the codec, so no turbofish is needed. If inference is ambiguous, annotate the
result as `Option<Point>`.

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

`Client` is cloneable and clones share one connection. Dropping a clone only
releases that clone; dropping the final clone triggers best-effort abortive
transport cleanup without waiting for admitted operations or transport
shutdown. Cleanup may continue asynchronously, and its errors cannot be
reported from `Drop`. Call `client.close().await?` when graceful draining and a
reported close error are required.

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

Inspect a structured value with `ValueKind` and borrowed accessors:

```rust
use openkache::{Client, Value, ValueKind};

#[tokio::main]
async fn main() -> openkache::Result<()> {
    let client = Client::connect("127.0.0.1:4433").await?;
    let profile = Value::map(vec![
        (Value::text("name"), Value::text("Ada")),
        (Value::text("visits"), Value::integer(3)),
        (Value::text("active"), Value::from(true)),
    ])
    .map_err(|error| openkache::Error::Core(error.to_string()))?;
    client.set("profile", profile).await?;

    let profile = match client.get("profile").await? {
        Some(value) => value,
        None => {
            return Err(openkache::Error::Core("profile is missing".into()));
        }
    };
    assert_eq!(profile.kind(), ValueKind::Map);
    assert_eq!(
        profile.map_get("name").and_then(Value::as_str),
        Some("Ada"),
    );
    assert_eq!(
        profile
            .map_get("visits")
            .and_then(Value::as_integer)
            .and_then(|value| value.as_i128()),
        Some(3),
    );
    assert_eq!(
        profile.map_get("active").and_then(Value::as_bool),
        Some(true),
    );
    assert_eq!(profile.map_get("name").and_then(Value::as_bool), None);

    client.close().await?;
    Ok(())
}
```

`map_get` and the typed accessors do not coerce values: a missing key or a
mismatched type returns `None`, so reading the text `name` with `as_bool`
above is safe.

For exact integer and float construction, use `Integer`, `Sign`,
`Float`, and `FloatWidth`. `ValueLimits` bounds bytes, depth, item count, and
integer magnitude. `ValueError` reports value validation and encoding errors;
use `ValueError::kind()` for its stable category.

`Result<T>` is the result alias returned by client methods.

### Results and errors

- Get methods return `Result<Option<T>>`; standard `Option` methods such as
  `unwrap`, `expect`, `unwrap_or`, and `unwrap_or_else` handle missing keys.
- `SetOutcome` is `Created` or `Replaced`.
- `SetOutcome::is_created` and `SetOutcome::is_replaced` inspect the write
  outcome without matching on its variants.
- `delete` returns `true` when an item existed and `false` otherwise.
- `Error::UnknownMutation` means a `set` or `delete` may have reached the
  server without a confirmed result. Its `Mutation` identifies the operation;
  do not replay it automatically. Use `Error::is_unknown_mutation`,
  `Error::mutation`, and `Error::kind` to inspect errors without matching on
  their payloads.
- `Error::Core` reports connection, protocol, key, value, or server failures.
- `Error::SerdeSerialize` and `Error::SerdeDeserialize` report Serde
  conversion failures before and after transport, respectively.
- `Error::CodecEncode` and `Error::CodecDecode` report application
  `ValueCodec` failures before and after transport, respectively.
- `Error::UnsupportedSetOutcome` reports a server result outside the
  unconditional-write API.

## More information

- [OpenKache on crates.io](https://crates.io/crates/openkache)
- [Rust API reference](https://docs.rs/openkache/latest/openkache/)
- [OpenKache repository](https://github.com/openkache/openkache)
