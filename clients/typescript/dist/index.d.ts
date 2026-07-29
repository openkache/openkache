/**
 * Promise-based Node.js client backed by the shared Rust OpenKache transport.
 */
import { type Value_Codec } from "./value-codec.js";
export type { Encoded_Value, Value_Codec, } from "./value-codec.js";
/**
 * Zstandard compression settings applied by the Rust value codec.
 */
export interface Zstandard_Options {
    /** Enables Zstandard compression before encryption. */
    readonly enabled?: boolean;
    /** Zstandard compression level from 1 through 22. */
    readonly level?: number;
    /** Values below this byte length bypass compression. */
    readonly minimum_input_size?: number;
    /** Compressed values must save at least this many bytes. */
    readonly minimum_savings?: number;
}
/**
 * Certificate identity presented to production servers that require mutual TLS.
 */
export interface Client_Identity {
    /** Client leaf certificate followed by intermediates, each encoded as DER or PEM. */
    readonly certificate_chain: readonly Uint8Array[];
    /** PKCS#1, SEC1, or PKCS#8 private key encoded as DER or PEM. */
    readonly private_key: Uint8Array;
}
/**
 * Native connection and complete request/response deadlines.
 */
export interface Client_Timeouts {
    /** Maximum duration for connection setup and the QUIC/TLS handshake. */
    readonly connect_ms?: number;
    /** Maximum duration for one complete cache operation. */
    readonly request_ms?: number;
}
/**
 * Connection settings for the Rust-backed Node.js client.
 */
export interface Client_Options {
    /** Server UDP socket address, such as `127.0.0.1:4433`. */
    readonly address: string;
    /** Server or CA certificate trusted for the QUIC connection, encoded as DER or PEM. */
    readonly certificate: Uint8Array;
    /** Exact 32-byte XChaCha20-Poly1305 key. */
    readonly encryption_key: Uint8Array;
    /** TLS server name. Defaults to `localhost`. */
    readonly server_name?: string;
    /** Client certificate and private key required by production mutual TLS. */
    readonly identity?: Client_Identity;
    /** Client-side compression settings. */
    readonly compression?: Zstandard_Options;
    /** Bounded connection and operation durations. */
    readonly timeouts?: Client_Timeouts;
    /** Optional Protobuf, FlatBuffers, or application value codecs. */
    readonly value_codecs?: readonly Value_Codec[];
    /** Explicit Rust helper executable path, primarily for custom packaging. */
    readonly helper_path?: string;
}
/**
 * Outcome of a successful `set` operation.
 */
export type Set_Outcome = "created" | "replaced" | "not_stored";
/**
 * Optional TTL and atomic existence condition for `set`.
 */
export interface Set_Options {
    /** Store only when the key is absent (`nx`) or present (`xx`). */
    readonly condition?: "nx" | "xx";
    /** Positive relative lifetime in milliseconds. */
    readonly ttl_ms?: number;
}
/**
 * Structured statistics returned by an administrator-authorized server.
 */
export interface Server_Stats {
    /** Storage implementation reported by the server. */
    readonly storage: string;
    /** Per-worker statistics encoded by the server. */
    readonly workers: readonly string[];
}
/**
 * Error returned by client validation, value codecs, helper, transport, or server failures.
 */
export declare class OpenKache_Error extends Error {
    readonly kind: "openkache_error";
    /**
     * Creates a stable client error.
     *
     * @param message - Human-readable failure description.
     * @param cause - Optional underlying failure.
     */
    constructor(message: string, cause?: unknown);
}
/**
 * Promise-based Node.js client that delegates native QUIC work to a Rust helper process.
 */
export declare class OpenKache_Client {
    #private;
    private constructor();
    /**
     * Connects through the packaged Rust helper without blocking the Node.js event loop.
     *
     * @param options - Address, trust, mTLS identity, encryption, and compression settings.
     * @returns A connected client that reuses one QUIC connection.
     * @throws {OpenKache_Error} When configuration, helper startup, TLS, or QUIC fails.
     */
    static connect(options: Client_Options): Promise<OpenKache_Client>;
    /**
     * Verifies that the server is reachable and speaks the expected protocol.
     *
     * @returns A promise that resolves after a valid `PONG`.
     * @throws {OpenKache_Error} When the client is closed or the operation fails.
     */
    ping(): Promise<void>;
    /**
     * Retrieves and codec-decodes a regular JavaScript object.
     *
     * @typeParam Value - Expected object shape selected by the caller.
     * @param key - Exact non-empty string or binary cache key.
     * @returns The decoded object, or `undefined` when the key does not exist.
     * @throws {OpenKache_Error} When transport, decryption, or decoding fails.
     */
    get<Value extends object = Record<string, unknown>>(key: string | Uint8Array): Promise<Value | undefined>;
    /**
     * Codec-encodes and stores a regular JavaScript object.
     *
     * @typeParam Value - Object shape to store.
     * @param key - Exact non-empty string or binary cache key.
     * @param value - Plain object accepted by a registered codec or built-in JSON.
     * @param options - Optional TTL and `nx` or `xx` existence condition.
     * @returns Whether the operation created, replaced, or did not store the key.
     * @throws {OpenKache_Error} When validation, encoding, transport, or storage fails.
     */
    set<Value extends object>(key: string | Uint8Array, value: Value, options?: Set_Options): Promise<Set_Outcome>;
    /**
     * Retrieves exact decrypted and decompressed bytes without envelope decoding.
     *
     * @param key - Exact non-empty string or binary cache key.
     * @returns Stored bytes, or `undefined` when the key does not exist.
     * @throws {OpenKache_Error} When the client is closed or the operation fails.
     */
    get_raw(key: string | Uint8Array): Promise<Uint8Array | undefined>;
    /**
     * Stores exact bytes without value-envelope encoding.
     *
     * @param key - Exact non-empty string or binary cache key.
     * @param value - Bytes to compress, encrypt, and store; empty values are supported.
     * @param options - Optional TTL and `nx` or `xx` existence condition.
     * @returns Whether the operation created, replaced, or did not store the key.
     * @throws {OpenKache_Error} When validation, transport, or storage fails.
     */
    set_raw(key: string | Uint8Array, value: Uint8Array, options?: Set_Options): Promise<Set_Outcome>;
    /**
     * Deletes a key.
     *
     * @param key - Exact non-empty string or binary cache key.
     * @returns `true` when the key existed and was deleted.
     * @throws {OpenKache_Error} When the client is closed or the operation fails.
     */
    delete(key: string | Uint8Array): Promise<boolean>;
    /**
     * Retrieves structured server statistics.
     *
     * @returns Validated storage and per-worker statistics.
     * @throws {OpenKache_Error} When authorization, transport, or response validation fails.
     */
    stats(): Promise<Server_Stats>;
    /**
     * Requests a server durability barrier.
     *
     * @returns A promise that resolves after every SSD worker flushes.
     * @throws {OpenKache_Error} When authorization, transport, or synchronization fails.
     */
    sync(): Promise<void>;
    /**
     * Closes the native connection and helper process. Repeated calls are safe.
     *
     * @returns A shared promise for helper shutdown.
     * @throws {OpenKache_Error} When the helper cannot acknowledge shutdown.
     */
    close(): Promise<void>;
}
//# sourceMappingURL=index.d.ts.map