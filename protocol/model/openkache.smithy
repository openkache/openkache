$version: "2"

namespace openkache.protocol

/// Values shared by every API using this transport.
@trait(selector: "service")
structure wireContract {
    @required
    maxPayloadBytes: Integer

    @required
    v1: WireV1
}

/// Versioned transport framing constants.
structure WireV1 {
    @required
    alpn: String

    @required
    requestCodeBytes: Integer

    @required
    responseCodeBytes: Integer

    @required
    maxVaruintBytes: Integer

    @required
    minVaruintBytes: Integer
}

@wireContract(
    maxPayloadBytes: 67108864,
    v1: {
        alpn: "openkache/1",
        requestCodeBytes: 1,
        responseCodeBytes: 1,
        maxVaruintBytes: 9,
        minVaruintBytes: 1,
    }
)
service OpenKache {
    version: "1"
}
