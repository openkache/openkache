# OpenKache transport profiles

OpenKache v1 defines identical frame bytes over two transport profiles:

- [QUIC](SPEC.md#quic-transport-profile)
- [TLS-over-TCP](SPEC.md#tls-over-tcp-transport-profile)

An implementation MUST support at least one profile and identify it. Maintained
OpenKache servers and clients support both. Both profiles use TLS 1.3,
`openkache/1`, and the mandatory `X25519MLKEM768` hybrid key agreement.

Transport negotiation, lane lifecycle, cancellation, and half-close behavior
are normative in [`SPEC.md`](SPEC.md#transport-and-version-negotiation).
