package io.openkache.client;

import java.util.Objects;

/**
 * Input shape for the experimental Smithy {@code Echo} operation.
 *
 * <p>The Java transport binding is still scaffold-only. This shape keeps the
 * public package contract aligned with the canonical client model while the
 * native transport is being designed.</p>
 *
 * @param message UTF-8 message to echo
 */
public record EchoInput(String message) {
    public EchoInput {
        Objects.requireNonNull(message, "message");
    }
}
