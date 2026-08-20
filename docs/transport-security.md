# Transport security profile

OpenKache transport profiles use TLS 1.3 and ALPN `openkache/1`. The required
key agreement is the hybrid `X25519MLKEM768` group. Classical-only negotiation,
plaintext TCP, and downgrade retries are non-conforming and fail closed.

Rustls-backed QUIC endpoints build one provider-neutral configuration that
advertises only the approved group. The quiche endpoint applies the equivalent
BoringSSL group name and rejects a BoringSSL build that does not provide it.
The reserved neqo backend is unavailable until its NSS integration can enforce
the same group.

The TLS-over-TCP implementation in this revision is a provider-neutral
one-lane boundary for the private integration layer. It is not yet wired into
the public `KacheServer` listener or command-line surface. The boundary keeps
TLS records and protocol frames bounded, permits finite pipelining under an
aggregate in-flight budget, rejects bytes after `close_notify`, treats EOF
without `close_notify` as unclean, and drains admitted responses before sending
the server close notification. A later listener task must preserve this
boundary rather than exposing plaintext socket bytes.

Certificate signatures remain configurable; the hybrid requirement applies to
the TLS key agreement, not to certificate authentication. Operators must use
certificate verification and client authentication according to their
deployment's trust requirements.
