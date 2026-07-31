// SPDX-FileCopyrightText: 2026 OpenStd Inc.
// SPDX-License-Identifier: Apache-2.0

namespace OpenKache;

/// <summary>
/// Controls whether a set operation may create or replace an item ID.
/// </summary>
public enum SetCondition
{
    /// <summary>Store the value regardless of whether the item ID exists.</summary>
    None,

    /// <summary>Store the value only when the item ID does not exist.</summary>
    IfAbsent,

    /// <summary>Store the value only when the item ID already exists.</summary>
    IfPresent,
}
