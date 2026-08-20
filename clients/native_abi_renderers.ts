/** Smithy-defined native ABI projections for managed client adapters. */

import type {
  Client_Contract,
  Native_Abi_Function,
  Native_Abi_Parameter,
  Native_Abi_Structure,
  Native_Abi_Type,
} from "./client_contract"
import {
  lower_camel_case,
  pascal_case,
  snake_case,
  swift_property_name,
  typescript_name,
} from "./generator_names"

export function native_abi_structure_class_name(structure_name: string): string {
  if (structure_name === "FfiNamespaceDescriptor") return "SmithyNativeDescriptor"
  return `SmithyNative${structure_name.replace(/^Ffi/, "")}`
}

function native_abi_java_type(
  type: Native_Abi_Type,
  structure_name?: string,
): string {
  switch (type) {
    case "client_pointer":
    case "result_pointer":
    case "request_pointer":
    case "u8_pointer":
      return "Pointer"
    case "struct_pointer":
      if (structure_name === undefined) {
        throw new Error("Java native struct pointer has no structure name")
      }
      return native_abi_structure_class_name(structure_name)
    case "size":
    case "uint64":
      return "long"
    case "int32":
    case "uint32":
      return "int"
    case "uint8":
      return "byte"
    case "void":
      return "void"
  }
}

function native_abi_kotlin_type(
  type: Native_Abi_Type,
  structure_name?: string,
): string {
  switch (type) {
    case "client_pointer":
    case "result_pointer":
    case "request_pointer":
    case "u8_pointer":
      return "Pointer?"
    case "struct_pointer":
      if (structure_name === undefined) {
        throw new Error("Kotlin native struct pointer has no structure name")
      }
      return native_abi_structure_class_name(structure_name)
    case "size":
    case "uint64":
      return "Long"
    case "int32":
    case "uint32":
      return "Int"
    case "uint8":
      return "Byte"
    case "void":
      return "Unit"
  }
}

function native_abi_dart_native_type(
  type: Native_Abi_Type,
  structure_name?: string,
): string {
  switch (type) {
    case "client_pointer":
      return "ffi.Pointer<SmithyNativeClient>"
    case "request_pointer":
      return "ffi.Pointer<SmithyNativeRequest>"
    case "struct_pointer":
      if (structure_name === undefined) {
        throw new Error("Dart native struct pointer has no structure name")
      }
      return `ffi.Pointer<${native_abi_structure_class_name(structure_name)}>`
    case "result_pointer":
      return "ffi.Pointer<SmithyNativeResult>"
    case "u8_pointer":
      return "ffi.Pointer<ffi.Uint8>"
    case "size":
      return "ffi.UintPtr"
    case "int32":
      return "ffi.Int32"
    case "uint32":
      return "ffi.Uint32"
    case "uint8":
      return "ffi.Uint8"
    case "uint64":
      return "ffi.Uint64"
    case "void":
      return "ffi.Void"
  }
}

function native_abi_dart_type(
  type: Native_Abi_Type,
  structure_name?: string,
): string {
  switch (type) {
    case "client_pointer":
      return "ffi.Pointer<SmithyNativeClient>"
    case "request_pointer":
      return "ffi.Pointer<SmithyNativeRequest>"
    case "struct_pointer":
      if (structure_name === undefined) {
        throw new Error("Dart native struct pointer has no structure name")
      }
      return `ffi.Pointer<${native_abi_structure_class_name(structure_name)}>`
    case "result_pointer":
      return "ffi.Pointer<SmithyNativeResult>"
    case "u8_pointer":
      return "ffi.Pointer<ffi.Uint8>"
    case "size":
    case "int32":
    case "uint32":
    case "uint8":
    case "uint64":
      return "int"
    case "void":
      return "void"
  }
}

export function native_abi_dart_suffix(name: string): string {
  const prefix = "openkache_client_"
  const suffix = name.startsWith(prefix) ? name.slice(prefix.length) : name
  return suffix === "free" ? "client_free" : suffix
}

function native_abi_structure_c_type_name(structure_name: string): string {
  if (structure_name === "FfiNamespaceDescriptor") {
    return "openkache_client_namespace_descriptor_t"
  }
  return `openkache_client_${snake_case(structure_name.replace(/^Ffi/, ""))}_t`
}

function native_abi_c_identifier(identifier: string): string {
  const normalized = snake_case(identifier)
  return normalized
    .replace(/_milliseconds\b/g, "_ms")
    .replace(/_namespace_id\b/g, "_namespace_id")
}

function native_abi_c_type(
  type: Native_Abi_Type,
  mutable: boolean,
  structure_name?: string,
): string {
  const pointer_qualifier = mutable ? "" : "const "
  switch (type) {
    case "client_pointer":
      return `${pointer_qualifier}openkache_client_t *`
    case "result_pointer":
      return `${pointer_qualifier}openkache_client_result_t *`
    case "request_pointer":
      return `${pointer_qualifier}openkache_client_request_t *`
    case "u8_pointer":
      return `${pointer_qualifier}uint8_t *`
    case "struct_pointer":
      if (structure_name === undefined) {
        throw new Error("C native struct pointer has no structure name")
      }
      return `${pointer_qualifier}${native_abi_structure_c_type_name(structure_name)} *`
    case "size":
      return "size_t"
    case "uint8":
      return "uint8_t"
    case "int32":
      return "int32_t"
    case "uint32":
      return "uint32_t"
    case "uint64":
      return "uint64_t"
    case "void":
      return "void"
  }
}

function native_abi_c_return_type(
  type: Native_Abi_Type,
  structure_name?: string,
): string {
  if (type === "u8_pointer") return "const uint8_t *"
  return native_abi_c_type(type, true, structure_name)
}

export function render_c_native_structures(contract: Client_Contract): string {
  return contract.ffi.native_abi_structures
    .map((structure) => {
      const fields = structure.fields
        .map(
          (field) =>
            `    ${native_abi_c_type(field.type, field.mutable, field.structure_name)} ${native_abi_c_identifier(field.name)};`,
        )
        .join("\n")
      return `typedef struct ${native_abi_structure_c_type_name(structure.name).replace(/_t$/, "")} {
${fields}
} ${native_abi_structure_c_type_name(structure.name)};`
    })
    .join("\n\n")
}

export function render_c_native_functions(contract: Client_Contract): string {
  return contract.ffi.native_abi_functions
    .map((function_) => {
      const ownership = function_.parameters
        .filter((parameter) => parameter.type.includes("pointer"))
        .map(
          (parameter) =>
            `${parameter.name}:${parameter.ownership}/${parameter.lifetime}`,
        )
        .join(", ")
      const return_ownership =
        `${function_.return_ownership}/${function_.return_lifetime}`
      const ownership_comment =
        `/* ${function_.name} ownership: return:${return_ownership}${
          ownership.length === 0 ? "" : `; parameters:${ownership}`
        }. */\n`
      const parameters = function_.parameters.length === 0
        ? "void"
        : function_.parameters
          .map(
            (parameter) =>
              `    ${native_abi_c_type(parameter.type, parameter.mutable, parameter.structure_name)} ${native_abi_c_identifier(parameter.name)}`,
          )
          .join(",\n")
      return `${ownership_comment}${native_abi_c_return_type(function_.return_type)} ${function_.name}(
${parameters}
);`
    })
    .join("\n\n")
}

export function native_abi_c_function_typedef_name(function_name: string): string {
  return `${function_name}_fn`
}

export function render_c_native_function_typedefs(contract: Client_Contract): string {
  return contract.ffi.native_abi_functions
    .map((function_) => {
      const parameters = function_.parameters.length === 0
        ? "void"
        : function_.parameters
          .map(
            (parameter) =>
              native_abi_c_type(parameter.type, parameter.mutable, parameter.structure_name),
          )
          .join(",\n")
      return `typedef ${native_abi_c_return_type(function_.return_type)} (*${native_abi_c_function_typedef_name(function_.name)})(
${parameters}
);`
    })
    .join("\n\n")
}

export function render_java_native_structure(
  structure: Native_Abi_Structure,
): string {
  const fields = structure.fields
    .map(
      (field) =>
        `    public ${native_abi_java_type(field.type, field.structure_name)} ${lower_camel_case(field.name)};`,
    )
    .join("\n")
  const field_order = structure.fields
    .map((field) => `"${lower_camel_case(field.name)}"`)
    .join(",\n        ")
  return `// Generated from the OpenKache Smithy client ABI contract. Do not edit.
package io.openkache.client.generated_local;

import com.sun.jna.Structure;
${structure.fields.some((field) => ["client_pointer", "result_pointer", "u8_pointer", "struct_pointer"].includes(field.type)) ? "import com.sun.jna.Pointer;" : ""}

/** C-compatible native ${structure.name} structure. */
@Structure.FieldOrder({
        ${field_order}
})
public final class ${native_abi_structure_class_name(structure.name)} extends Structure {
${fields}
}
`
}

function render_kotlin_native_structure(
  structure: Native_Abi_Structure,
): string {
  const fields = structure.fields
    .map(
      (field) =>
        `    @JvmField var ${lower_camel_case(field.name)}: ${native_abi_kotlin_type(field.type, field.structure_name)} = ${native_abi_kotlin_default(field.type)}`,
    )
    .join("\n")
  const field_order = structure.fields
    .map((field) => `"${lower_camel_case(field.name)}"`)
    .join(",\n        ")
  return `/** C-compatible native ${structure.name} structure. */
@Structure.FieldOrder(
        ${field_order}
)
public class ${native_abi_structure_class_name(structure.name)} : Structure() {
${fields}
}`
}

function native_abi_kotlin_default(type: Native_Abi_Type): string {
  switch (type) {
    case "client_pointer":
    case "result_pointer":
    case "request_pointer":
    case "u8_pointer":
    case "struct_pointer":
      return "null"
    case "size":
    case "int32":
    case "uint32":
    case "uint64":
    case "uint8":
      return "0"
    case "void":
      throw new Error("Kotlin native structure field cannot be void")
  }
}

function native_abi_dart_field(
  field: Native_Abi_Parameter,
): string {
  const name = lower_camel_case(field.name)
  switch (field.type) {
    case "client_pointer":
      return `  external ffi.Pointer<SmithyNativeClient> ${name};`
    case "result_pointer":
      return `  external ffi.Pointer<SmithyNativeResult> ${name};`
    case "request_pointer":
      return `  external ffi.Pointer<SmithyNativeRequest> ${name};`
    case "u8_pointer":
      return `  external ffi.Pointer<ffi.Uint8> ${name};`
    case "struct_pointer":
      return `  external ffi.Pointer<${native_abi_structure_class_name(field.structure_name!)}> ${name};`
    case "size":
      return `  @ffi.UintPtr()
  external int ${name};`
    case "uint8":
      return `  @ffi.Uint8()
  external int ${name};`
    case "int32":
      return `  @ffi.Int32()
  external int ${name};`
    case "uint32":
      return `  @ffi.Uint32()
  external int ${name};`
    case "uint64":
      return `  @ffi.Uint64()
  external int ${name};`
  }
}

function render_dart_native_structure(
  structure: Native_Abi_Structure,
): string {
  const fields = structure.fields.map(native_abi_dart_field).join("\n\n")
  return `final class ${native_abi_structure_class_name(structure.name)} extends ffi.Struct {
${fields}
}`
}

export function required_native_structure(
  contract: Client_Contract,
  name: string,
): Native_Abi_Structure {
  const structure = contract.ffi.native_abi_structures.find(
    (candidate) => candidate.name === name,
  )
  if (structure === undefined) {
    throw new Error(`Smithy native ABI structure ${name} is required`)
  }
  return structure
}

export function render_java_native_connect_options(contract: Client_Contract): string {
  return render_java_native_structure(required_native_structure(contract, "FfiConnectOptions"))
}

export function render_java_native_descriptor(contract: Client_Contract): string {
  const fields = contract.ffi.namespace_descriptor_fields
  const field_order = fields.map((field) => `"${lower_camel_case(field.name)}"`).join(",\n        ")
  const declarations = fields
    .map(
      (field) =>
        `    public ${native_abi_java_type(field.rust_type === "u64" ? "uint64" : "uint32")} ${lower_camel_case(field.name)};`,
    )
    .join("\n")
  return `// Generated from the OpenKache Smithy contract. Do not edit.
package io.openkache.client.generated_local;

import com.sun.jna.Structure;

/** C-compatible namespace descriptor returned by the native ABI. */
@Structure.FieldOrder({
        ${field_order}
})
public final class SmithyNativeDescriptor extends Structure {
${declarations}
}
`
}

export function render_java_native_api(contract: Client_Contract): string {
  const methods = contract.ffi.native_abi_functions
    .map((function_) => {
      const parameters = function_.parameters
        .map(
          (parameter) =>
            `${native_abi_java_type(parameter.type, parameter.structure_name)} ${parameter.name}`,
        )
        .join(",\n            ")
      return `    ${native_abi_java_type(function_.return_type)} ${function_.name}(${parameters});`
    })
    .join("\n\n")
  return `// Generated from the OpenKache Smithy client ABI contract. Do not edit.
package io.openkache.client.generated_local;

import com.sun.jna.Library;
import com.sun.jna.Pointer;

/** Native function declarations shared by managed language adapters. */
public interface SmithyNativeApi extends Library {
${methods}
}
`
}

export function render_kotlin_native_api(contract: Client_Contract): string {
  const methods = contract.ffi.native_abi_functions
    .map((function_) => {
      const parameters = function_.parameters
        .map(
          (parameter) =>
            `${parameter.name}: ${native_abi_kotlin_type(parameter.type, parameter.structure_name)}`,
        )
        .join(",\n            ")
      return `    fun ${function_.name}(${parameters}): ${native_abi_kotlin_type(function_.return_type)}`
    })
    .join("\n\n")
  const fields = contract.ffi.namespace_descriptor_fields
    .map(
      (field) =>
        `    @JvmField var ${lower_camel_case(field.name)}: ${field.rust_type === "u64" ? "Long" : "Int"} = 0`,
    )
    .join("\n")
  const field_order = contract.ffi.namespace_descriptor_fields
    .map((field) => `"${lower_camel_case(field.name)}"`)
    .join(",\n        ")
  return `// Generated from the OpenKache Smithy client ABI contract. Do not edit.
package io.openkache.client.generated_local

import com.sun.jna.Library
import com.sun.jna.Pointer
import com.sun.jna.Structure

/** Native function declarations shared by managed language adapters. */
public interface SmithyNativeApi : Library {
${methods}
}

/** C-compatible namespace descriptor returned by the native ABI. */
@Structure.FieldOrder(
        ${field_order}
)
public class SmithyNativeDescriptor : Structure() {
${fields}
}

${contract.ffi.native_abi_structures.map(render_kotlin_native_structure).join("\n\n")}
`
}

export function render_dart_native_api(contract: Client_Contract): string {
  const fields = contract.ffi.namespace_descriptor_fields
    .map((field) => {
      const annotation = field.rust_type === "u64" ? "Uint64" : "Uint32"
      return `  @ffi.${annotation}()
  external int ${lower_camel_case(field.name)};`
    })
    .join("\n\n")
  const typedefs = contract.ffi.native_abi_functions
    .map((function_) => {
      const suffix = pascal_case(native_abi_dart_suffix(function_.name))
      const native_parameters = function_.parameters
        .map((parameter) => `  ${native_abi_dart_native_type(parameter.type, parameter.structure_name)}`)
        .join(",\n")
      const dart_parameters = function_.parameters
        .map((parameter) => `  ${native_abi_dart_type(parameter.type, parameter.structure_name)}`)
        .join(",\n")
      const native_signature = function_.parameters.length === 0
        ? "Function()"
        : `Function(\n${native_parameters}\n)`
      const dart_signature = function_.parameters.length === 0
        ? "Function()"
        : `Function(\n${dart_parameters}\n)`
      return `typedef Smithy${suffix}Native = ${native_abi_dart_native_type(function_.return_type)} ${native_signature};

typedef Smithy${suffix}Dart = ${native_abi_dart_type(function_.return_type)} ${dart_signature};`
    })
    .join("\n\n")
  const constructor_initializers = contract.ffi.native_abi_functions
    .map((function_) => {
      const suffix = pascal_case(native_abi_dart_suffix(function_.name))
      const field = lower_camel_case(native_abi_dart_suffix(function_.name))
      return `        ${field} = library.lookupFunction<Smithy${suffix}Native, Smithy${suffix}Dart>(
          '${function_.name}',
        )`
    })
    .join(",\n")
  const members = contract.ffi.native_abi_functions
    .map((function_) => {
      const field = lower_camel_case(native_abi_dart_suffix(function_.name))
      return `  final ${native_abi_dart_type(function_.return_type)} Function(${function_.parameters
        .map((parameter) => native_abi_dart_type(parameter.type, parameter.structure_name))
        .join(", ")}) ${field};`
    })
    .join("\n")
  return `// Generated from the OpenKache Smithy client ABI contract. Do not edit.

import 'dart:ffi' as ffi;
import 'dart:io';

final class SmithyNativeClient extends ffi.Opaque {}

final class SmithyNativeResult extends ffi.Opaque {}

final class SmithyNativeRequest extends ffi.Opaque {}

final class SmithyNativeDescriptor extends ffi.Struct {
${fields}
}

${contract.ffi.native_abi_structures.map(render_dart_native_structure).join("\n\n")}

${typedefs}

/** Native function bindings shared by managed language adapters. */
final class SmithyNativeApi {
  SmithyNativeApi(ffi.DynamicLibrary library)
: ${constructor_initializers};

${members}

  static SmithyNativeApi open(String? configuredPath) {
    final path = configuredPath ??
        Platform.environment['OPENKACHE_CLIENT_NATIVE'] ??
        switch (Platform.operatingSystem) {
          'linux' => 'libopenkache_client_core.so',
          'macos' => 'libopenkache_client_core.dylib',
          'windows' => 'openkache_client_core.dll',
          _ => throw UnsupportedError(
              'unsupported platform \${Platform.operatingSystem}',
            ),
        };
    return SmithyNativeApi(ffi.DynamicLibrary.open(path));
  }
}
`
}
function native_abi_go_field_name(function_name: string): string {
  const suffix = native_abi_dart_suffix(function_name)
  return suffix === "abi_version" ? "abi" : suffix
}

/** Renders the generated C preprocessor list consumed by the Go cgo loader. */
export function render_go_native_abi(contract: Client_Contract): string {
  const render_function_list = (
    macro_name: string,
    functions: readonly Native_Abi_Function[],
  ): string => {
    if (functions.length === 0) {
      return `#define ${macro_name}(X)`
    }
    const entries = functions
      .map((function_, index) => {
        const continuation = index === functions.length - 1 ? "" : " \\"
        return `    X(${native_abi_go_field_name(function_.name)}, ${native_abi_c_function_typedef_name(function_.name)}, "${function_.name}")${continuation}`
      })
      .join("\n")
    return `#define ${macro_name}(X) \\
${entries}`
  }
  const required_functions = contract.ffi.native_abi_functions.filter(
    (function_) => !function_.optional,
  )
  const optional_functions = contract.ffi.native_abi_functions.filter(
    (function_) => function_.optional,
  )
  return `/* Generated from the OpenKache Smithy client ABI contract. Do not edit. */
#ifndef OPENKACHE_SMITHY_NATIVE_ABI_H
#define OPENKACHE_SMITHY_NATIVE_ABI_H

#include <openkache/client_abi.h>

/*
 * Expands to (field name, function-pointer type, exported symbol) triples.
 * The Go cgo loader uses this list for both its native-library state and
 * symbol registration, so adding an ABI function to Smithy cannot silently
 * leave the loader stale.
 */
${render_function_list("OPENKACHE_SMITHY_NATIVE_FUNCTIONS", contract.ffi.native_abi_functions)}

/*
 * Required symbols must be present in every ABI implementation. Optional
 * symbols are exposed separately so loaders can preserve older-library
 * compatibility without maintaining a second hand-written list.
 */
${render_function_list("OPENKACHE_SMITHY_NATIVE_REQUIRED_FUNCTIONS", required_functions)}
${render_function_list("OPENKACHE_SMITHY_NATIVE_OPTIONAL_FUNCTIONS", optional_functions)}

#endif /* OPENKACHE_SMITHY_NATIVE_ABI_H */
`
}

function native_abi_python_structure_name(structure_name: string): string {
  if (structure_name === "FfiNamespaceDescriptor") {
    return "SmithyFFINamespaceDescriptor"
  }
  return `SmithyNative${structure_name.replace(/^Ffi/, "")}`
}

function native_abi_python_type(
  type: Native_Abi_Type,
  structure_name?: string,
): string {
  switch (type) {
    case "client_pointer":
      return "_CLIENT_POINTER"
    case "result_pointer":
      return "_RESULT_POINTER"
    case "request_pointer":
      return "_REQUEST_POINTER"
    case "u8_pointer":
      return "_U8_POINTER"
    case "struct_pointer":
      if (structure_name === undefined) {
        throw new Error("Python native struct pointer has no structure name")
      }
      return `_ctypes.POINTER(${native_abi_python_structure_name(structure_name)})`
    case "size":
      return "_ctypes.c_size_t"
    case "uint8":
      return "_ctypes.c_uint8"
    case "int32":
      return "_ctypes.c_int32"
    case "uint32":
      return "_ctypes.c_uint32"
    case "uint64":
      return "_ctypes.c_uint64"
    case "void":
      return "None"
  }
}

function native_abi_python_attribute(function_name: string): string {
  const suffix = native_abi_dart_suffix(function_name)
  switch (suffix) {
    case "connect":
      return "connect_legacy"
    case "connect_ex":
      return "connect"
    case "result_data_length":
      return "result_length"
    default:
      return suffix
  }
}

/** Renders ctypes classes and signatures from the Smithy native ABI contract. */
export function render_python_native_abi(contract: Client_Contract): string {
  const structures = contract.ffi.native_abi_structures
    .map((structure) => {
      const fields = structure.fields
        .map(
          (field) =>
            `        ("${snake_case(field.name)}", ${native_abi_python_type(field.type, field.structure_name)}),`,
        )
        .join("\n")
      return `class ${native_abi_python_structure_name(structure.name)}(_ctypes.Structure):
    """C-compatible ${structure.name} layout generated from Smithy."""

    _fields_ = [
${fields}
    ]`
    })
    .join("\n\n")
  const functions = contract.ffi.native_abi_functions
    .map((function_) => {
      const arguments_ = function_.parameters.length === 0
        ? "()"
        : `(
${function_.parameters
  .map(
    (parameter) =>
      `        ${native_abi_python_type(parameter.type, parameter.structure_name)},`,
  )
  .join("\n")}
    )`
      return `    "${native_abi_python_attribute(function_.name)}": (
        "${function_.name}",
        ${arguments_},
        ${native_abi_python_type(function_.return_type)},
    ),`
    })
    .join("\n")
  return `# Generated from the OpenKache Smithy client ABI contract. Do not edit.

import ctypes as _ctypes

from .smithy_contract import SmithyFFINamespaceDescriptor

_CLIENT_POINTER = _ctypes.c_void_p
_RESULT_POINTER = _ctypes.c_void_p
_REQUEST_POINTER = _ctypes.c_void_p
_U8 = _ctypes.c_uint8
_U8_POINTER = _ctypes.POINTER(_U8)

${structures}

SMITHY_NATIVE_FUNCTIONS = {
${functions}
}
`
}

function native_abi_swift_structure_name(structure_name: string): string {
  if (structure_name === "FfiNamespaceDescriptor") {
    return "Smithy_Native_Namespace_Descriptor"
  }
  return `Smithy_Native_${typescript_name(structure_name.replace(/^Ffi/, ""))}`
}

function native_abi_swift_type(
  type: Native_Abi_Type,
  mutable: boolean,
  structure_name?: string,
): string {
  const pointer = mutable ? "UnsafeMutablePointer" : "UnsafePointer"
  switch (type) {
    case "client_pointer":
    case "result_pointer":
    case "request_pointer":
      return "OpaquePointer?"
    case "u8_pointer":
      return `${pointer}<UInt8>?`
    case "struct_pointer":
      if (structure_name === undefined) {
        throw new Error("Swift native struct pointer has no structure name")
      }
      return `${pointer}<${native_abi_swift_structure_name(structure_name)}>?`
    case "size":
      return "Int"
    case "uint8":
      return "UInt8"
    case "int32":
      return "Int32"
    case "uint32":
      return "UInt32"
    case "uint64":
      return "UInt64"
    case "void":
      return "Void"
  }
}

function native_abi_swift_function_name(function_name: string): string {
  const suffix = native_abi_dart_suffix(function_name)
  const names: Readonly<Record<string, string>> = {
    abi_version: "nativeAbiVersion",
    connect: "nativeConnectLegacy",
    connect_ex: "nativeConnect",
    connect_with_options: "nativeConnectWithOptions",
    execute: "nativeExecute",
    execute_raw: "nativeExecuteRaw",
    execute_with_options: "nativeExecuteWithOptions",
    execute_raw_with_options: "nativeExecuteRawWithOptions",
    execute_scoped: "nativeExecuteScoped",
    namespace_open: "nativeNamespaceOpen",
    namespace_update_policy: "nativeNamespaceUpdatePolicy",
    namespace_delete: "nativeNamespaceDelete",
    namespace_descriptor_decode: "nativeNamespaceDescriptorDecode",
    connection_state: "nativeConnectionState",
    result_kind: "nativeResultKind",
    result_data: "nativeResultData",
    result_data_length: "nativeResultDataLength",
    result_take_client: "nativeTakeClient",
    result_free: "nativeFreeResult",
    client_free: "nativeFreeClient",
  }
  const function_name_value = names[suffix]
  if (function_name_value !== undefined) return function_name_value
  return `native${pascal_case(suffix)}`
}

/** Renders Swift FFI declarations from the Smithy native ABI contract. */
export function render_swift_native_abi(contract: Client_Contract): string {
  const structures = contract.ffi.native_abi_structures.map((structure) => {
    const fields = structure.fields
      .map(
        (field) =>
          `  var ${swift_property_name(field.name)}: ${native_abi_swift_type(field.type, field.mutable, field.structure_name)} = ${native_abi_swift_default(field.type)}`,
      )
      .join("\n")
    return `/// C-compatible ${structure.name} layout generated from Smithy.
internal struct ${native_abi_swift_structure_name(structure.name)} {
${fields}
}`
  })
  const functions = contract.ffi.native_abi_functions
    .map((function_) => {
      const name = native_abi_swift_function_name(function_.name)
      const parameters = function_.parameters
        .map(
          (parameter) =>
            `  _ ${swift_property_name(parameter.name)}: ${native_abi_swift_type(parameter.type, parameter.mutable, parameter.structure_name)}`,
        )
        .join(",\n")
      const return_type = native_abi_swift_type(
        function_.return_type,
        function_.return_type === "u8_pointer" ? false : true,
      )
      const declaration = function_.parameters.length === 0
        ? `internal func ${name}()`
        : `internal func ${name}(
${parameters}
)`
      return `@_silgen_name("${function_.name}")
${declaration}${return_type === "Void" ? "" : ` -> ${return_type}`}`
    })
    .join("\n\n")
  return `// Generated from the OpenKache Smithy client ABI contract. Do not edit.

import Foundation

typealias Smithy_Native_Client_Pointer = OpaquePointer
typealias Smithy_Native_Result_Pointer = OpaquePointer

${structures.join("\n\n")}

${functions}
`
}

function native_abi_swift_default(type: Native_Abi_Type): string {
  switch (type) {
    case "client_pointer":
    case "result_pointer":
    case "request_pointer":
    case "u8_pointer":
    case "struct_pointer":
      return "nil"
    case "size":
    case "int32":
    case "uint8":
    case "uint32":
    case "uint64":
      return "0"
    case "void":
      throw new Error("Swift native structure field cannot be void")
  }
}

function native_abi_csharp_structure_name(structure_name: string): string {
  if (structure_name === "FfiNamespaceDescriptor") {
    return "Protocol.FfiNamespaceDescriptor"
  }
  if (structure_name === "FfiConnectOptions") {
    return "ConnectOptions"
  }
  return `Native${pascal_case(snake_case(structure_name.replace(/^Ffi/, "")))}`
}

function native_abi_csharp_scalar_type(type: Native_Abi_Type): string {
  switch (type) {
    case "client_pointer":
    case "result_pointer":
    case "request_pointer":
    case "u8_pointer":
    case "struct_pointer":
      return "IntPtr"
    case "size":
      return "nuint"
    case "uint8":
      return "byte"
    case "int32":
      return "int"
    case "uint32":
      return "uint"
    case "uint64":
      return "ulong"
    case "void":
      return "void"
  }
}

function native_abi_csharp_parameter_type(
  parameter: Native_Abi_Parameter,
): string {
  if (parameter.type !== "struct_pointer") {
    return native_abi_csharp_scalar_type(parameter.type)
  }
  const modifier = parameter.mutable ? "out " : "ref "
  return `${modifier}${native_abi_csharp_structure_name(parameter.structure_name!)}`
}

function native_abi_csharp_identifier(identifier: string): string {
  const camel = lower_camel_case(identifier)
  return camel.length === 0
    ? camel
    : `${camel[0]?.toUpperCase()}${camel.slice(1)}`
}

/** Renders .NET P/Invoke declarations from the Smithy native ABI contract. */
export function render_csharp_native_abi(contract: Client_Contract): string {
  required_native_structure(contract, "FfiConnectOptions")
  const native_structures = contract.ffi.native_abi_structures
    .filter((structure) => structure.name !== "FfiNamespaceDescriptor")
    .map((structure) => {
      const fields = structure.fields
        .map(
          (field) =>
            `        internal ${native_abi_csharp_scalar_type(field.type)} ${native_abi_csharp_identifier(field.name)};`,
        )
        .join("\n")
      return `    [StructLayout(LayoutKind.Sequential)]
    internal struct ${native_abi_csharp_structure_name(structure.name)}
    {
${fields}
    }`
    })
    .join("\n\n")
  const functions = contract.ffi.native_abi_functions
    .map((function_) => {
      const parameters = function_.parameters.length === 0
        ? ""
        : function_.parameters
          .map(
            (parameter) =>
              `        ${native_abi_csharp_parameter_type(parameter)} ${native_abi_csharp_identifier(parameter.name)}`,
          )
          .join(",\n")
      return `[DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern ${native_abi_csharp_scalar_type(function_.return_type)} ${function_.name}(
${parameters}
    );`
    })
    .join("\n\n")
  return `// Generated from the OpenKache Smithy client ABI contract. Do not edit.
using System;
using System.Runtime.InteropServices;

namespace OpenKache;

internal static partial class NativeMethods
{
${native_structures}

${functions}
}
`
}
