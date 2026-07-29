/**
 * Promise-based Node.js client backed by the shared Rust OpenKache transport.
 */
import { spawn, } from "node:child_process";
import { fileURLToPath } from "node:url";
import { decode_helper_error, encode_close_request, encode_connect_request, encode_execute_request, Helper_Response_Decoder, OPERATION_DELETE, OPERATION_GET, OPERATION_PING, OPERATION_SET, OPERATION_STATS, OPERATION_SYNC, RESULT_CONNECTED, RESULT_CREATED, RESULT_DELETED, RESULT_NOT_DELETED, RESULT_NOT_FOUND, RESULT_NOT_STORED, RESULT_OK, RESULT_REPLACED, RESULT_VALUE, } from "./helper-protocol.js";
import { Value_Codec_Registry, } from "./value-codec.js";
const EMPTY_BYTES = new Uint8Array();
const MAX_VALUE_BYTES = 16 * 1024 * 1024;
const ENCRYPTION_OVERHEAD_BYTES = 40;
const MAX_PLAINTEXT_BYTES = MAX_VALUE_BYTES - ENCRYPTION_OVERHEAD_BYTES;
const MAX_STDERR_BYTES = 64 * 1024;
const TEXT_ENCODER = new TextEncoder();
const TEXT_DECODER = new TextDecoder("utf-8", { fatal: true });
const CLIENT_FINALIZER = new FinalizationRegistry((helper) => {
    helper.kill();
});
/**
 * Error returned by client validation, value codecs, helper, transport, or server failures.
 */
export class OpenKache_Error extends Error {
    kind = "openkache_error";
    /**
     * Creates a stable client error.
     *
     * @param message - Human-readable failure description.
     * @param cause - Optional underlying failure.
     */
    constructor(message, cause) {
        super(message, cause === undefined ? undefined : { cause });
        this.name = "OpenKache_Error";
    }
}
/**
 * Promise-based Node.js client that delegates native QUIC work to a Rust helper process.
 */
export class OpenKache_Client {
    #helper;
    #channel;
    #value_codecs;
    #next_request_id = 1;
    #close_promise;
    #closed = false;
    constructor(helper, value_codecs) {
        this.#helper = helper;
        this.#value_codecs = value_codecs;
        this.#channel = {
            decoder: new Helper_Response_Decoder(),
            pending: new Map(),
            closed: false,
            stderr: "",
        };
        helper.stdout.on("data", (chunk) => {
            receive_helper_bytes(this.#channel, helper, chunk);
        });
        helper.stderr.on("data", (chunk) => {
            this.#channel.stderr = append_stderr(this.#channel.stderr, chunk);
        });
        helper.on("error", (error) => {
            close_failed_helper(this.#channel, helper, as_openkache_error(error));
        });
        helper.on("exit", (code, signal) => {
            if (this.#channel.closed)
                return;
            let truncated_error = "";
            try {
                this.#channel.decoder.finish();
            }
            catch (error) {
                truncated_error = `: ${error_message(error)}`;
            }
            const status = signal === null ? `status ${code ?? "unknown"}` : `signal ${signal}`;
            const stderr = this.#channel.stderr.length === 0 ? "" : `: ${this.#channel.stderr.trim()}`;
            close_failed_helper(this.#channel, helper, new OpenKache_Error(`OpenKache native helper exited with ${status}${stderr}${truncated_error}`));
        });
    }
    /**
     * Connects through the packaged Rust helper without blocking the Node.js event loop.
     *
     * @param options - Address, trust, mTLS identity, encryption, and compression settings.
     * @returns A connected client that reuses one QUIC connection.
     * @throws {OpenKache_Error} When configuration, helper startup, TLS, or QUIC fails.
     */
    static async connect(options) {
        validate_options(options);
        let value_codecs;
        try {
            value_codecs = new Value_Codec_Registry(options.value_codecs ?? []);
        }
        catch (error) {
            throw new OpenKache_Error(`value codec configuration failed: ${error_message(error)}`, error);
        }
        const helper_path = options.helper_path ?? default_helper_path();
        const helper = spawn(helper_path, [], {
            stdio: ["pipe", "pipe", "pipe"],
            windowsHide: true,
        });
        const client = new OpenKache_Client(helper, value_codecs);
        const compression = options.compression ?? {};
        const helper_options = {
            address: options.address,
            server_name: options.server_name ?? "localhost",
            certificate: options.certificate,
            identity: options.identity,
            encryption_key: options.encryption_key,
            compression_enabled: compression.enabled !== false,
            compression_level: compression.level ?? 1,
            minimum_input_size: compression.minimum_input_size ?? 1_024,
            minimum_savings: compression.minimum_savings ?? 64,
        };
        try {
            const response = await client.#request((request_id) => encode_connect_request(request_id, helper_options));
            if (response.result_kind !== RESULT_CONNECTED) {
                throw unexpected_result("connect", response.result_kind);
            }
            CLIENT_FINALIZER.register(client, helper, client);
            return client;
        }
        catch (error) {
            helper.kill();
            throw as_openkache_error(error);
        }
    }
    /**
     * Verifies that the server is reachable and speaks the expected protocol.
     *
     * @returns A promise that resolves after a valid `PONG`.
     * @throws {OpenKache_Error} When the client is closed or the operation fails.
     */
    async ping() {
        await this.#expect_kind(OPERATION_PING, EMPTY_BYTES, RESULT_OK);
    }
    /**
     * Retrieves and codec-decodes a regular JavaScript object.
     *
     * @typeParam Value - Expected object shape selected by the caller.
     * @param key - Exact non-empty string or binary cache key.
     * @returns The decoded object, or `undefined` when the key does not exist.
     * @throws {OpenKache_Error} When transport, decryption, or decoding fails.
     */
    async get(key) {
        const bytes = await this.get_raw(key);
        if (bytes === undefined)
            return undefined;
        try {
            return this.#value_codecs.decode(bytes);
        }
        catch (error) {
            throw new OpenKache_Error(`value decoding failed: ${error_message(error)}`, error);
        }
    }
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
    async set(key, value, options = {}) {
        let bytes;
        try {
            bytes = this.#value_codecs.encode(value);
        }
        catch (error) {
            throw new OpenKache_Error(`value encoding failed: ${error_message(error)}`, error);
        }
        validate_value_length(bytes);
        return this.#set_owned_bytes(key, bytes, options);
    }
    /**
     * Retrieves exact decrypted and decompressed bytes without envelope decoding.
     *
     * @param key - Exact non-empty string or binary cache key.
     * @returns Stored bytes, or `undefined` when the key does not exist.
     * @throws {OpenKache_Error} When the client is closed or the operation fails.
     */
    async get_raw(key) {
        const response = await this.#execute(OPERATION_GET, key, EMPTY_BYTES);
        if (response.result_kind === RESULT_NOT_FOUND)
            return undefined;
        if (response.result_kind !== RESULT_VALUE) {
            throw unexpected_result("GET", response.result_kind);
        }
        return response.payload;
    }
    /**
     * Stores exact bytes without value-envelope encoding.
     *
     * @param key - Exact non-empty string or binary cache key.
     * @param value - Bytes to compress, encrypt, and store; empty values are supported.
     * @param options - Optional TTL and `nx` or `xx` existence condition.
     * @returns Whether the operation created, replaced, or did not store the key.
     * @throws {OpenKache_Error} When validation, transport, or storage fails.
     */
    async set_raw(key, value, options = {}) {
        validate_value_length(value);
        return this.#set_owned_bytes(key, value.slice(), options);
    }
    /**
     * Deletes a key.
     *
     * @param key - Exact non-empty string or binary cache key.
     * @returns `true` when the key existed and was deleted.
     * @throws {OpenKache_Error} When the client is closed or the operation fails.
     */
    async delete(key) {
        const response = await this.#execute(OPERATION_DELETE, key, EMPTY_BYTES);
        if (response.result_kind === RESULT_DELETED)
            return true;
        if (response.result_kind === RESULT_NOT_DELETED)
            return false;
        throw unexpected_result("DELETE", response.result_kind);
    }
    /**
     * Retrieves structured server statistics.
     *
     * @returns Validated storage and per-worker statistics.
     * @throws {OpenKache_Error} When authorization, transport, or response validation fails.
     */
    async stats() {
        const response = await this.#execute(OPERATION_STATS, EMPTY_BYTES, EMPTY_BYTES);
        if (response.result_kind !== RESULT_VALUE) {
            throw unexpected_result("STATS", response.result_kind);
        }
        try {
            return parse_stats(TEXT_DECODER.decode(response.payload));
        }
        catch (error) {
            throw new OpenKache_Error(`STATS decoding failed: ${error_message(error)}`, error);
        }
    }
    /**
     * Requests a server durability barrier.
     *
     * @returns A promise that resolves after every SSD worker flushes.
     * @throws {OpenKache_Error} When authorization, transport, or synchronization fails.
     */
    async sync() {
        await this.#expect_kind(OPERATION_SYNC, EMPTY_BYTES, RESULT_OK);
    }
    /**
     * Closes the native connection and helper process. Repeated calls are safe.
     *
     * @returns A shared promise for helper shutdown.
     * @throws {OpenKache_Error} When the helper cannot acknowledge shutdown.
     */
    close() {
        this.#close_promise ??= this.#close_once();
        return this.#close_promise;
    }
    async #set_owned_bytes(key, bytes, options) {
        validate_set_options(options);
        const response = await this.#execute(OPERATION_SET, key, bytes, options);
        const outcomes = {
            [RESULT_CREATED]: "created",
            [RESULT_REPLACED]: "replaced",
            [RESULT_NOT_STORED]: "not_stored",
        };
        const outcome = outcomes[response.result_kind];
        if (outcome === undefined)
            throw unexpected_result("SET", response.result_kind);
        return outcome;
    }
    async #expect_kind(operation, key, expected_kind) {
        const response = await this.#execute(operation, key, EMPTY_BYTES);
        if (response.result_kind !== expected_kind) {
            throw unexpected_result("operation", response.result_kind);
        }
    }
    #execute(operation, key, value, set_options = {}) {
        const key_bytes = owned_key_bytes(key, operation === OPERATION_GET ||
            operation === OPERATION_SET ||
            operation === OPERATION_DELETE);
        const condition = set_options.condition === undefined
            ? 0
            : SET_CONDITIONS[set_options.condition];
        return this.#request((request_id) => encode_execute_request(request_id, {
            operation,
            condition,
            ttl_ms: set_options.ttl_ms ?? 0,
            key: key_bytes,
            value,
        }));
    }
    #request(encode_request, allow_closing = false) {
        if (this.#closed ||
            this.#channel.closed ||
            (!allow_closing && this.#close_promise !== undefined)) {
            return Promise.reject(new OpenKache_Error("client is closed"));
        }
        const request_id = this.#next_request_id;
        this.#next_request_id += 1;
        let frame;
        try {
            frame = encode_request(request_id);
        }
        catch (error) {
            return Promise.reject(as_openkache_error(error));
        }
        return new Promise((resolve, reject) => {
            this.#channel.pending.set(request_id, { resolve, reject });
            this.#helper.stdin.write(frame, (error) => {
                if (error === null || error === undefined)
                    return;
                const pending = this.#channel.pending.get(request_id);
                if (pending === undefined)
                    return;
                this.#channel.pending.delete(request_id);
                pending.reject(as_openkache_error(error));
            });
        });
    }
    async #close_once() {
        try {
            const response = await this.#request((request_id) => encode_close_request(request_id), true);
            if (response.result_kind !== RESULT_OK) {
                throw unexpected_result("close", response.result_kind);
            }
        }
        finally {
            this.#closed = true;
            this.#channel.closed = true;
            CLIENT_FINALIZER.unregister(this);
            this.#helper.kill();
            fail_pending_requests(this.#channel, new OpenKache_Error("client is closed"));
        }
    }
}
const SET_CONDITIONS = {
    nx: 1,
    xx: 2,
};
function receive_helper_bytes(channel, helper, chunk) {
    try {
        for (const response of channel.decoder.push(chunk)) {
            receive_helper_response(channel, response);
        }
    }
    catch (error) {
        close_failed_helper(channel, helper, as_openkache_error(error));
    }
}
function receive_helper_response(channel, response) {
    const pending = channel.pending.get(response.request_id);
    if (pending === undefined)
        return;
    channel.pending.delete(response.request_id);
    if (response.ok) {
        pending.resolve(response);
    }
    else {
        try {
            pending.reject(new OpenKache_Error(decode_helper_error(response.payload)));
        }
        catch (error) {
            pending.reject(as_openkache_error(error));
        }
    }
}
function close_failed_helper(channel, helper, error) {
    if (channel.closed)
        return;
    channel.closed = true;
    helper.kill();
    fail_pending_requests(channel, error);
}
function fail_pending_requests(channel, error) {
    for (const pending of channel.pending.values()) {
        pending.reject(error);
    }
    channel.pending.clear();
}
function append_stderr(existing, chunk) {
    const combined = existing + new TextDecoder().decode(chunk);
    return combined.length <= MAX_STDERR_BYTES
        ? combined
        : combined.slice(combined.length - MAX_STDERR_BYTES);
}
function owned_key_bytes(key, required) {
    const bytes = typeof key === "string" ? TEXT_ENCODER.encode(key) : key.slice();
    if (required && bytes.byteLength === 0) {
        throw new OpenKache_Error("key must not be empty");
    }
    return bytes;
}
function validate_options(options) {
    if (options.address.length === 0)
        throw new OpenKache_Error("address must not be empty");
    if (options.certificate.byteLength === 0) {
        throw new OpenKache_Error("certificate must not be empty");
    }
    if (options.encryption_key.byteLength !== 32) {
        throw new OpenKache_Error(`encryption_key must contain 32 bytes, got ${options.encryption_key.byteLength}`);
    }
    if (options.server_name !== undefined && options.server_name.length === 0) {
        throw new OpenKache_Error("server_name must not be empty");
    }
    if (options.helper_path !== undefined && options.helper_path.length === 0) {
        throw new OpenKache_Error("helper_path must not be empty");
    }
    validate_identity(options.identity);
    validate_compression(options.compression);
}
function validate_identity(identity) {
    if (identity === undefined)
        return;
    if (identity.certificate_chain.length === 0) {
        throw new OpenKache_Error("identity.certificate_chain must not be empty");
    }
    if (identity.certificate_chain.length > 0xffff) {
        throw new OpenKache_Error("identity.certificate_chain contains too many certificates");
    }
    for (const certificate of identity.certificate_chain) {
        if (certificate.byteLength === 0) {
            throw new OpenKache_Error("identity certificates must not be empty");
        }
    }
    if (identity.private_key.byteLength === 0) {
        throw new OpenKache_Error("identity.private_key must not be empty");
    }
}
function validate_compression(compression) {
    if (compression === undefined)
        return;
    if (compression.level !== undefined &&
        (!Number.isInteger(compression.level) ||
            compression.level < 1 ||
            compression.level > 22)) {
        throw new OpenKache_Error("compression.level must be an integer from 1 through 22");
    }
    for (const [name, value] of [
        ["minimum_input_size", compression.minimum_input_size],
        ["minimum_savings", compression.minimum_savings],
    ]) {
        if (value !== undefined && (!Number.isSafeInteger(value) || value < 0)) {
            throw new OpenKache_Error(`compression.${name} must be a non-negative safe integer`);
        }
    }
}
function validate_value_length(value) {
    if (value.byteLength > MAX_PLAINTEXT_BYTES) {
        throw new OpenKache_Error(`value contains ${value.byteLength} bytes, maximum is ${MAX_PLAINTEXT_BYTES}`);
    }
}
function validate_set_options(options) {
    if (options.ttl_ms !== undefined &&
        (!Number.isSafeInteger(options.ttl_ms) || options.ttl_ms <= 0)) {
        throw new OpenKache_Error("ttl_ms must be a positive safe integer");
    }
}
function is_regular_object(value) {
    if (value === null || typeof value !== "object" || Array.isArray(value))
        return false;
    if (value instanceof Uint8Array)
        return false;
    const prototype = Object.getPrototypeOf(value);
    return prototype === Object.prototype || prototype === null;
}
function parse_stats(text) {
    const value = JSON.parse(text);
    if (!is_regular_object(value)) {
        throw new Error("response is not an object");
    }
    const candidate = value;
    if (typeof candidate.storage !== "string") {
        throw new Error("response.storage is not a string");
    }
    if (!Array.isArray(candidate.workers) ||
        !candidate.workers.every((worker) => typeof worker === "string")) {
        throw new Error("response.workers is not a string array");
    }
    return {
        storage: candidate.storage,
        workers: candidate.workers,
    };
}
function unexpected_result(operation, kind) {
    return new OpenKache_Error(`${operation} returned unexpected native result ${kind}`);
}
function as_openkache_error(error) {
    return error instanceof OpenKache_Error
        ? error
        : new OpenKache_Error(error_message(error), error);
}
function error_message(error) {
    return error instanceof Error ? error.message : String(error);
}
function default_helper_path() {
    if (process.platform !== "linux" || process.arch !== "x64") {
        throw new OpenKache_Error(`packaged helper supports Linux x64, got ${process.platform} ${process.arch}; ` +
            "provide helper_path for a custom build");
    }
    return fileURLToPath(new URL("../target/native/x86_64-unknown-linux-musl/release/openkache-client-helper", import.meta.url));
}
//# sourceMappingURL=index.js.map