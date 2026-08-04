// SPDX-FileCopyrightText: 2026 OpenStd Inc.
// SPDX-License-Identifier: Apache-2.0

namespace OpenKache;

/// <summary>
/// Compatibility aliases for the generated Smithy set-condition shape.
/// </summary>
/// <remarks>
/// The canonical type is <see cref="Smithy.SetCondition"/>. The nullable
/// <see cref="SetOptions.Condition"/> property represents an unconditional set with
/// <see cref="Any"/>.
/// </remarks>
public static class SetCondition
{
    /// <summary>Store the value regardless of whether the item ID exists.</summary>
    public static Smithy.SetCondition? Any => null;

    /// <summary>Store the value only when the item ID does not exist.</summary>
    public const Smithy.SetCondition IfAbsent = Smithy.SetCondition.IfAbsent;

    /// <summary>Store the value only when the item ID already exists.</summary>
    public const Smithy.SetCondition IfPresent = Smithy.SetCondition.IfPresent;
}
