package io.openkache.client;

/**
 * Failure reported by the shared Rust client-core ABI.
 */
public final class EchoClientException extends RuntimeException {
    public EchoClientException(String message) {
        super(message);
    }

    public EchoClientException(String message, Throwable cause) {
        super(message, cause);
    }
}
