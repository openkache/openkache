/** Shared identifier projections used by contract extraction and renderers. */

/** Converts snake_case model identifiers to PascalCase source identifiers. */
export function pascal_case(identifier: string): string {
  return identifier
    .split("_")
    .map((part) => {
      const normalized = part.toLowerCase()
      return normalized.length === 0
        ? ""
        : `${normalized[0]?.toUpperCase()}${normalized.slice(1)}`
    })
    .join("")
}

/** Converts a model identifier to lower-camel source spelling. */
export function lower_camel_case(identifier: string): string {
  const pascal =
    /[_-]/.test(identifier) || identifier === identifier.toUpperCase()
      ? pascal_case(identifier)
      : `${identifier[0]?.toUpperCase()}${identifier.slice(1)}`
  return pascal.length === 0
    ? pascal
    : `${pascal[0]?.toLowerCase()}${pascal.slice(1)}`
}

/** Converts a model identifier to the generated TypeScript/Swift type spelling. */
export function typescript_name(identifier: string): string {
  return snake_case(identifier)
    .split("_")
    .map((part) => `${part[0]?.toUpperCase()}${part.slice(1)}`)
    .join("_")
}

/** Converts Smithy/CamelCase identifiers to the stable wire snake_case form. */
export function snake_case(identifier: string): string {
  return identifier
    .replace(/([a-z0-9])([A-Z])/g, "$1_$2")
    .replace(/([A-Z]+)([A-Z][a-z])/g, "$1_$2")
    .replace(/-/g, "_")
    .toLowerCase()
}

/** Converts a model identifier to the PascalCase spelling used by Swift. */
export function swift_name(identifier: string): string {
  return identifier
    .split(/[_-]/)
    .filter((part) => part.length > 0)
    .map((part) => {
      const normalized =
        part === part.toUpperCase()
          ? part.toLowerCase()
          : `${part[0]?.toLowerCase()}${part.slice(1)}`
      return `${normalized[0]?.toUpperCase()}${normalized.slice(1)}`
    })
    .join("")
}

/** Converts a model identifier to the lower-camel Swift property spelling. */
export function swift_property_name(identifier: string): string {
  const name = swift_name(identifier)
  return name.length === 0 ? name : `${name[0]?.toLowerCase()}${name.slice(1)}`
}

/** Converts a model member to the Go exported spelling used by generated APIs. */
export function go_exported_name(identifier: string): string {
  return pascal_case(snake_case(identifier))
    .replace(/Id$/, "ID")
    .replace(/^Ttl/, "TTL")
    .replace(/^Json$/, "JSON")
}
