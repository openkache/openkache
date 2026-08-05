package io.openkache.client;

import java.util.concurrent.CompletionStage;

/**
 * Smithy operation surface implemented by the Java adapter.
 */
public interface OpenKacheClient {
    /**
     * Sends an experimental UTF-8 message and returns the echoed message.
     *
     * @param input operation input
     * @return asynchronous operation result
     */
    CompletionStage<EchoOutput> echo(EchoInput input);
}
