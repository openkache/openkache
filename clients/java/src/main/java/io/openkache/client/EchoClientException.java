package io.openkache.client;

/**
 * @deprecated Use {@link OpenKacheClientException}; retained for source compatibility.
 */
@Deprecated
public final class EchoClientException extends OpenKacheClientException {
    public EchoClientException(String message) {
        super(message);
    }

    public EchoClientException(String message, Throwable cause) {
        super(message, cause);
    }
}
