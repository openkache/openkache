// Generated from the OpenKache Smithy contract. Do not edit.
package io.openkache.client.generated;

/** Smithy operation types for Java adapters. */
public final class SmithyApi {
    private SmithyApi() {}

    public enum SetCondition {
        IfAbsent("if_absent"),
        IfPresent("if_present");
        public final String value;
        SetCondition(String value) { this.value = value; }
    }

    public enum SetOutcome {
        Created("created"),
        Replaced("replaced"),
        NotStored("not_stored");
        public final String value;
        SetOutcome(String value) { this.value = value; }
    }

    public record DeleteInput(
        byte[] itemId,
        byte[] mutationId
    ) {}

    public record DeleteOutput(
        boolean deleted
    ) {}

    public record GetInput(
        byte[] itemId
    ) {}

    public record GetOutput(
        byte[] value
    ) {}

    public record PingInput(

    ) {}

    public record PingOutput(

    ) {}

    public record SetInput(
        byte[] itemId,
        byte[] value,
        String condition,
        Long ttlMilliseconds,
        byte[] mutationId
    ) {}

    public record SetOutput(
        String outcome
    ) {}

    public record StatsInput(

    ) {}

    public record StatsOutput(
        String json
    ) {}

    public record SyncInput(

    ) {}

    public record SyncOutput(

    ) {}

    public interface OpenKacheApi {
        java.util.concurrent.CompletionStage<PingOutput> ping(PingInput input);
        java.util.concurrent.CompletionStage<GetOutput> get(GetInput input);
        java.util.concurrent.CompletionStage<SetOutput> set(SetInput input);
        java.util.concurrent.CompletionStage<DeleteOutput> delete(DeleteInput input);
        java.util.concurrent.CompletionStage<StatsOutput> stats(StatsInput input);
        java.util.concurrent.CompletionStage<SyncOutput> sync(SyncInput input);
    }
}
