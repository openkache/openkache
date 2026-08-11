package io.openkache.client;

/**
 * Failure reported by the shared Rust client-core ABI or a generated operation.
 */
public class OpenKacheClientException extends RuntimeException {
    public OpenKacheClientException(String message) {
        super(message);
    }

    public OpenKacheClientException(String message, Throwable cause) {
        super(message, cause);
    }
}
