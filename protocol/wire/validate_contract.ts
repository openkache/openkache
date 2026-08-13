/** Shared structural validation for transport-contract extraction. */

import type { Wire_Entry } from "../wire_types"

export type Json_Object = Readonly<Record<string, unknown>>

export function object_value(value: unknown, location: string): Json_Object {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error(`${location} must be an object`)
  }
  return value as Json_Object
}

export function object_member(
  object: Json_Object,
  member: string,
  location: string,
): Json_Object {
  return object_value(object[member], `${location}.${member}`)
}

export function array_member(
  object: Json_Object,
  member: string,
  location: string,
): readonly unknown[] {
  const value = object[member]
  if (!Array.isArray(value)) throw new Error(`${location}.${member} must be an array`)
  return value
}

export function string_member(
  object: Json_Object,
  member: string,
  location: string,
): string {
  const value = object[member]
  if (typeof value !== "string" || value.length === 0) {
    throw new Error(`${location}.${member} must be a non-empty string`)
  }
  return value
}

export function integer_member(
  object: Json_Object,
  member: string,
  location: string,
  minimum = 0,
  maximum = Number.MAX_SAFE_INTEGER,
): number {
  const value = object[member]
  if (
    typeof value !== "number" ||
    !Number.isSafeInteger(value) ||
    value < minimum ||
    value > maximum
  ) {
    throw new Error(
      `${location}.${member} must be an integer from ${minimum} through ${maximum}`,
    )
  }
  return value
}

export function trait_value(
  shape: Json_Object,
  trait_id: string,
  location: string,
): Json_Object {
  const traits = object_member(shape, "traits", location)
  return object_member(traits, trait_id, `${location}.traits`)
}

export function optional_enum_value(
  shape: Json_Object,
  location: string,
): string | undefined {
  const traits = object_member(shape, "traits", location)
  const value = traits["smithy.api#enumValue"]
  if (value === undefined) return undefined
  return string_member(traits, "smithy.api#enumValue", `${location}.traits`)
}

export function unique_wire_values(
  entries: readonly Wire_Entry[],
  kind: string,
): void {
  const names = new Set<string>()
  const texts = new Set<string>()
  const values = new Set<number>()
  for (const entry of entries) {
    if (names.has(entry.name)) throw new Error(`duplicate ${kind} name ${entry.name}`)
    if (entry.text !== undefined && texts.has(entry.text)) {
      throw new Error(`duplicate ${kind} enum value ${entry.text}`)
    }
    if (values.has(entry.value)) {
      throw new Error(`duplicate ${kind} wire value ${entry.value}`)
    }
    names.add(entry.name)
    if (entry.text !== undefined) texts.add(entry.text)
    values.add(entry.value)
  }
}
