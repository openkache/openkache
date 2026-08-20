# Transport security profile

OpenKache transport profiles use TLS 1.3 and ALPN `openkache/1`. The required
key agreement is the hybrid `X25519MLKEM768` group. Classical-only negotiation,
plaintext TCP, and downgrade retries are non-conforming and fail closed.

Rustls-backed QUIC endpoints build one provider-neutral configuration that
advertises only the approved group. The quiche endpoint applies the equivalent
BoringSSL group name and rejects a BoringSSL build that does not provide it.
The reserved neqo backend is unavailable until its NSS integration can enforce
the same group.

The maintained `KacheServer` listener exposes this profile alongside QUIC. It
accepts one TLS-over-TCP lane per connection, keeps TLS records and protocol
frames bounded, permits finite pipelining under an aggregate in-flight budget,
rejects bytes after `close_notify`, treats EOF without `close_notify` as
unclean, and drains admitted responses before sending the server close
notification. The listener uses the selected network runtime's TCP adapter and
never exposes plaintext socket bytes. By default it binds the same IP and port
as the QUIC listener; configure `[tcp].listen` to select another address.

Certificate signatures remain configurable; the hybrid requirement applies to
the TLS key agreement, not to certificate authentication. Operators must use
certificate verification and client authentication according to their
deployment's trust requirements.
