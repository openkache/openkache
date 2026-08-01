// SPDX-FileCopyrightText: 2026 OpenStd Inc.
// SPDX-License-Identifier: Apache-2.0

namespace OpenKache;

/// <summary>
/// Compatibility aliases for the generated Smithy set-outcome shape.
/// </summary>
/// <remarks>
/// The canonical type is <see cref="Smithy.SetOutcome"/>. Client set methods return that
/// generated type directly.
/// </remarks>
public static class SetOutcome
{
    /// <summary>The item ID did not previously exist.</summary>
    public const Smithy.SetOutcome Created = Smithy.SetOutcome.Created;

    /// <summary>The item ID previously existed and was replaced.</summary>
    public const Smithy.SetOutcome Replaced = Smithy.SetOutcome.Replaced;

    /// <summary>The value was not stored because its existence condition failed.</summary>
    public const Smithy.SetOutcome NotStored = Smithy.SetOutcome.NotStored;
}
