package io.openkache.client;

import java.util.Objects;
import java.util.concurrent.CompletionStage;
import java.util.function.Supplier;

/**
 * @deprecated Use {@link Client} and the generated Smithy operation DTOs.
 * This facade keeps the experimental ECHO convenience method source-compatible.
 */
@Deprecated
public final class EchoClient implements OpenKacheClient, AutoCloseable {
    private final Client delegate;

    private EchoClient(Client delegate) {
        this.delegate = delegate;
    }

    public static EchoClient connect(
        String address,
        String serverName,
        byte[] certificate,
        byte[] dataProtectionKey) {
        return new EchoClient(
            Client.connect(address, serverName, certificate, dataProtectionKey));
    }

    /** Sends one message and decodes the strict UTF-8 response. */
    public CompletionStage<String> echo(String message) {
        return delegate.echo(new EchoInput(Objects.requireNonNull(message, "message")))
            .thenApply(EchoOutput::message);
    }

    @Override
    public <T> CompletionStage<T> smithySubmit(Supplier<T> operation) {
        return delegate.smithySubmit(operation);
    }

    @Override
    public NativeResult smithyExecute(
        int operation,
        byte[] applicationKey,
        byte[] value,
        int setCondition,
        long ttlMilliseconds) {
        return delegate.smithyExecute(
            operation,
            applicationKey,
            value,
            setCondition,
            ttlMilliseconds);
    }

    @Override
    public NativeResult smithyExecuteScoped(
        int operation,
        long namespaceId,
        byte[] itemId,
        byte[] value,
        int setFlags,
        long ttlMilliseconds) {
        return delegate.smithyExecuteScoped(
            operation,
            namespaceId,
            itemId,
            value,
            setFlags,
            ttlMilliseconds);
    }

    @Override
    public NativeResult smithyNamespaceOpen(
        byte[] name,
        boolean createIfMissing,
        int policyFlags,
        long ttlMilliseconds) {
        return delegate.smithyNamespaceOpen(
            name,
            createIfMissing,
            policyFlags,
            ttlMilliseconds);
    }

    @Override
    public NativeResult smithyNamespaceUpdatePolicy(
        long namespaceId,
        long expectedRevision,
        int policyFlags,
        long ttlMilliseconds) {
        return delegate.smithyNamespaceUpdatePolicy(
            namespaceId,
            expectedRevision,
            policyFlags,
            ttlMilliseconds);
    }

    @Override
    public NativeResult smithyNamespaceDelete(long namespaceId, long expectedRevision) {
        return delegate.smithyNamespaceDelete(namespaceId, expectedRevision);
    }

    @Override
    public NamespaceDescriptor smithyDecodeDescriptor(byte[] payload) {
        return delegate.smithyDecodeDescriptor(payload);
    }

    @Override
    public String smithyDecodeUtf8(byte[] payload, String operation) {
        return delegate.smithyDecodeUtf8(payload, operation);
    }

    @Override
    public void close() {
        delegate.close();
    }
}
