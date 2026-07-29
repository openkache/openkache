// SPDX-FileCopyrightText: 2026 OpenStd Inc.
// SPDX-License-Identifier: Apache-2.0

namespace OpenKache;

/// <summary>
/// Reports OpenKache connection, protocol, timeout, and server failures.
/// </summary>
public sealed class OpenKacheException : Exception
{
    /// <summary>
    /// Stable machine-readable error identifier.
    /// </summary>
    public string Code { get; }

    internal OpenKacheException(
        string code,
        string message,
        Exception? innerException = null)
        : base($"[{code}] {message}", innerException)
    {
        Code = code;
    }
}
