/**
 * Runtime-neutral envelope and codec registry for cross-language values.
 */
/**
 * Encoded payload and logical type returned by a custom value codec.
 */
export interface Encoded_Value {
    /** Cross-language type identifier, such as `acme.profile.v1`. */
    readonly type_name: string;
    /** Codec-specific bytes stored inside the OpenKache value envelope. */
    readonly payload: Uint8Array;
}
/**
 * Pluggable cross-language object codec.
 *
 * A Protobuf or FlatBuffers integration can own a schema registry internally.
 * The stored envelope carries `encoding` and `type_name`, so cache operations
 * do not need positional schema arguments.
 */
export interface Value_Codec {
    /** Stable cross-language encoding identifier, such as `protobuf`. */
    readonly encoding: string;
    /**
     * Reports whether this codec owns a value.
     *
     * @param value - Regular JavaScript object supplied to `set`.
     * @returns Whether `encode` should serialize this value.
     */
    can_encode(value: object): boolean;
    /**
     * Serializes an owned value.
     *
     * @param value - Value accepted by `can_encode`.
     * @returns Logical type metadata and encoded payload bytes.
     * @throws {Error} When the value cannot be encoded.
     */
    encode(value: object): Encoded_Value;
    /**
     * Deserializes bytes selected by the stored envelope.
     *
     * @param type_name - Cross-language logical type stored with the payload.
     * @param payload - Exact codec-specific payload bytes.
     * @returns A regular JavaScript object.
     * @throws {Error} When the type is unknown or the payload is invalid.
     */
    decode(type_name: string, payload: Uint8Array): object;
}
/**
 * Selects codecs for writes and routes stored envelopes for reads.
 */
export declare class Value_Codec_Registry {
    #private;
    /**
     * Creates a registry with built-in JSON fallback.
     *
     * @param codecs - Optional Protobuf, FlatBuffers, or application codecs.
     * @throws {Error} When encoding identifiers are invalid or duplicated.
     */
    constructor(codecs: readonly Value_Codec[]);
    /**
     * Encodes a regular object using one custom codec or JSON fallback.
     *
     * @param value - Regular JavaScript object supplied to `set`.
     * @returns Versioned, self-describing OpenKache value bytes.
     * @throws {Error} When codec selection is ambiguous or encoding fails.
     */
    encode(value: object): Uint8Array;
    /**
     * Decodes an OpenKache value envelope through its registered codec.
     *
     * @param bytes - Decrypted and decompressed value bytes.
     * @returns A regular JavaScript object.
     * @throws {Error} When the envelope or selected codec is invalid.
     */
    decode(bytes: Uint8Array): object;
}
//# sourceMappingURL=value-codec.d.ts.map