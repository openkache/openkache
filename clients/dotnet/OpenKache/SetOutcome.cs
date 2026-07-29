// SPDX-FileCopyrightText: 2026 OpenStd Inc.
// SPDX-License-Identifier: Apache-2.0

namespace OpenKache;

/// <summary>
/// Describes the result of a successful set operation.
/// </summary>
public enum SetOutcome
{
    /// <summary>The key did not previously exist.</summary>
    Created,

    /// <summary>The key previously existed and was replaced.</summary>
    Replaced,
}
