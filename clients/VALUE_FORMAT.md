# OpenKache value envelope

OpenKache object APIs store a versioned envelope around a codec-specific
payload. This lets clients share JSON, Protobuf, FlatBuffers, or another agreed
format without passing a schema to every `get` and `set`.

The server treats the complete envelope as opaque bytes. Envelope encoding
happens before compression and encryption, and decoding happens after
decryption and decompression.

## Binary contract

All integers are unsigned and big-endian.

| Offset | Size | Field |
|---|---:|---|
| 0 | 4 | Magic and envelope version: `4f 4b 56 01` |
| 4 | 2 | Encoding identifier byte length |
| 6 | 2 | Type name byte length |
| 8 | variable | UTF-8 encoding identifier |
| variable | variable | UTF-8 logical type name |
| variable | remaining | Codec-specific payload |

Encoding identifiers contain lowercase ASCII letters, digits, dots, and
hyphens, start with a letter, and contain at most 64 bytes. Type names and codec
payloads are opaque to the envelope. A raw byte API bypasses the envelope.

The Rust client's `value_envelope` module is the reference implementation. Its
`encode` function produces owned bytes, while `decode` validates the envelope
and borrows its metadata and payload without copying.

## Standard encodings

`json` is built in. Its type name is empty and its payload is exactly one UTF-8
JSON object following RFC 8259. The common values are objects with string keys,
dense arrays, strings, finite numbers, booleans, and null. TypeScript omits
object properties whose value is `undefined`.

The identifiers `protobuf` and `flatbuffers` are intended for codec plugins:

- A Protobuf codec stores the fully qualified message name as the type name and
  the binary message as the payload.
- A FlatBuffers codec stores an application-stable table identifier as the type
  name and the finished FlatBuffer as the payload.

These codecs maintain a local mapping from type names to generated schemas.
The envelope embeds the type identity, not a complete schema descriptor.
Registering schemas once avoids positional schema arguments and avoids copying
large descriptors into every cache value. An application that requires
self-contained descriptors can define a codec whose payload includes them.

Application codecs should use a stable, cross-language encoding identifier and
document their type-name and payload contracts.

## Client behavior

For `set`, a client checks registered codecs first and rejects ambiguous
matches. If no custom codec accepts the object, it uses the JSON codec. For
`get`, the client reads the envelope and routes the payload by encoding and type
name. Unknown encodings fail explicitly; callers can still retrieve their exact
bytes through the raw API.

Language clients implement object conversion with their native JSON,
Protobuf, FlatBuffers, or generated-code runtime. Fixed conformance vectors
keep those implementations byte-compatible with the Rust reference. The
format does not depend on JavaScript or a particular serializer.

## Browser preparation

The TypeScript envelope and codec registry use `Uint8Array`, `DataView`,
`TextEncoder`, `TextDecoder`, and `JSON`, with no Node.js imports. A future
WebTransport client can reuse the same value layer while replacing only the
current Node.js helper transport.
