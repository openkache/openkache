// SPDX-FileCopyrightText: 2026 OpenStd Inc.
// SPDX-License-Identifier: Apache-2.0

namespace OpenKache;

/// <summary>
/// Controls whether a set operation may create or replace a key.
/// </summary>
public enum SetCondition
{
    /// <summary>Store the value regardless of whether the key exists.</summary>
    None,

    /// <summary>Store the value only when the key does not exist.</summary>
    Nx,

    /// <summary>Store the value only when the key already exists.</summary>
    Xx,
}
