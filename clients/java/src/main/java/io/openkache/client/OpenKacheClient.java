package io.openkache.client;

/**
 * Public Java adapter surface for the supported Smithy operations.
 *
 * <p>The generated Smithy shapes are emitted into the same package during the
 * build. Keeping this small adapter interface separate lets the experimental
 * implementation expose only the operation it actually supports while the
 * generated {@code SmithyOpenKacheApi} remains the complete model surface.</p>
 */
public interface OpenKacheClient extends SmithyEchoApi {}
