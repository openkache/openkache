package io.openkache.client;

import java.util.Objects;

/**
 * Output shape for the experimental Smithy {@code Echo} operation.
 *
 * @param message UTF-8 message returned by the server
 */
public record EchoOutput(String message) {
    public EchoOutput {
        Objects.requireNonNull(message, "message");
    }
}
