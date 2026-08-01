/** Canonical JSON value validation shared by the TypeScript adapter. */

export type Json_Value =
  | null
  | boolean
  | number
  | string
  | readonly Json_Value[]
  | Json_Object

export interface Json_Object {
  readonly [key: string]: Json_Value
}

/** Validates a dense, finite JSON value. */
export function assert_json_value(value: unknown): asserts value is Json_Value {
  validate_json_value(value, "$", new WeakSet())
}

function validate_json_value(
  value: unknown,
  path: string,
  ancestors: WeakSet<object>,
): void {
  if (value === null || typeof value === "string" || typeof value === "boolean") return
  if (typeof value === "number") {
    if (!Number.isFinite(value)) throw new Error(`${path} must contain a finite JSON number`)
    return
  }
  if (Array.isArray(value)) {
    validate_container(value, path, ancestors, (): void => {
      for (const key of Object.keys(value)) {
        if (!is_array_index_key(key, value.length)) {
          throw new Error(`${path} must not contain enumerable property "${key}"`)
        }
      }
      if (
        Object.getOwnPropertySymbols(value).some((symbol) =>
          Object.prototype.propertyIsEnumerable.call(value, symbol),
        )
      ) {
        throw new Error(`${path} must not contain enumerable symbol properties`)
      }
      for (let index = 0; index < value.length; index += 1) {
        if (!(index in value)) throw new Error(`${path}[${index}] must not be sparse`)
        validate_json_value(value[index], `${path}[${index}]`, ancestors)
      }
    })
    return
  }
  if (is_regular_object(value)) {
    validate_container(value, path, ancestors, (): void => {
      if (
        Object.getOwnPropertySymbols(value).some((symbol) =>
          Object.prototype.propertyIsEnumerable.call(value, symbol),
        )
      ) {
        throw new Error(`${path} must not contain enumerable symbol properties`)
      }
      for (const [key, child] of Object.entries(value)) {
        validate_json_value(child, `${path}.${key}`, ancestors)
      }
    })
    return
  }
  throw new Error(`${path} contains an unsupported JSON value`)
}

function validate_container(
  value: object,
  path: string,
  ancestors: WeakSet<object>,
  validate_children: () => void,
): void {
  if (ancestors.has(value)) throw new Error(`${path} contains a cyclic reference`)
  ancestors.add(value)
  try {
    validate_children()
  } finally {
    ancestors.delete(value)
  }
}

function is_regular_object(value: unknown): value is Record<string, unknown> {
  if (value === null || typeof value !== "object" || Array.isArray(value)) return false
  const prototype = Object.getPrototypeOf(value)
  return prototype === Object.prototype || prototype === null
}

function is_array_index_key(key: string, length: number): boolean {
  if (!/^(0|[1-9][0-9]*)$/.test(key)) return false
  const index = Number(key)
  return Number.isSafeInteger(index) && index >= 0 && index < length
}
