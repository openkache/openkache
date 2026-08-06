library;

import 'dart:convert';
import 'dart:ffi' as ffi;

import 'package:ffi/ffi.dart';

import 'generated_local/smithy_api.dart';
import 'generated_local/smithy_contract.dart';
import 'generated_local/smithy_native_api.dart';

part 'generated_local/smithy_operations.dart';

/// Failure reported by the shared Rust client-core ABI.
final class OpenKacheClientException implements Exception {
  const OpenKacheClientException(this.message, [this.cause]);

  final String message;
  final Object? cause;

  @override
  String toString() => cause == null ? message : '$message: $cause';
}

/// Dart adapter surface for the complete generated Smithy API.
abstract interface class OpenKacheClient implements SmithyOpenKacheApi {}

/// Rust-backed Dart client implementing every generated Smithy operation.
///
/// QUIC, TLS, framing, retries, namespace handling, and response ownership
/// remain in the shared Rust client core. This adapter only marshals Dart DTOs
/// through the stable C ABI.
final class Client with SmithyGeneratedOperations implements OpenKacheClient {
  Client._(this._api, this._handle);

  final SmithyNativeApi _api;
  ffi.Pointer<SmithyNativeClient> _handle;
  bool _closed = false;

  /// Connects to an OpenKache server through the shared native client core.
  factory Client.connect({
    required String address,
    required String serverName,
    List<int> certificate = const <int>[],
    required List<int> dataProtectionKey,
    String? libraryPath,
  }) {
    if (dataProtectionKey.length != 32) {
      throw ArgumentError.value(
        dataProtectionKey.length,
        'dataProtectionKey',
        'must contain exactly 32 bytes',
      );
    }
    final SmithyNativeApi api;
    try {
      api = SmithyNativeApi.open(libraryPath);
    } on Object catch (error) {
      throw OpenKacheClientException(
        'failed to load OpenKache native client',
        error,
      );
    }
    if (api.abiVersion() != smithyFfiAbiVersion) {
      throw const OpenKacheClientException(
        'unsupported OpenKache native ABI version',
      );
    }
    final addressBuffer = _Buffer(utf8.encode(address));
    final serverBuffer = _Buffer(utf8.encode(serverName));
    final certificateBuffer = _Buffer(certificate);
    final keyBuffer = _Buffer(dataProtectionKey);
    try {
      final result = _readResult(
        api,
        api.connect(
          addressBuffer.pointer,
          addressBuffer.length,
          serverBuffer.pointer,
          serverBuffer.length,
          certificateBuffer.pointer,
          certificateBuffer.length,
          keyBuffer.pointer,
          keyBuffer.length,
          0,
          smithyDefaultZstandardLevel,
          smithyDefaultZstandardMinimumInputBytes,
          smithyDefaultZstandardMinimumSavingsBytes,
          smithyDefaultConnectTimeoutMilliseconds,
          smithyDefaultRequestTimeoutMilliseconds,
        ),
        takeClient: true,
      );
      final client = result.client;
      if (result.kind != smithyResultConnected || client == ffi.nullptr) {
        throw const OpenKacheClientException(
          'native client did not return a connected handle',
        );
      }
      return Client._(api, client);
    } finally {
      addressBuffer.close();
      serverBuffer.close();
      certificateBuffer.close();
      keyBuffer.close();
    }
  }

  /// Releases the shared native client handle.
  void close() {
    if (_closed) return;
    _closed = true;
    _api.clientFree(_handle);
    _handle = ffi.nullptr;
  }

  Future<T> _run<T>(T Function() operation) => Future<T>(operation);

  _NativeResult _invoke(
    int operation,
    List<int> applicationKey,
    List<int> value, {
    int setCondition = smithySetConditionAny,
    int ttlMilliseconds = 0,
  }) {
    final keyBuffer = _Buffer(applicationKey);
    final valueBuffer = _Buffer(value);
    try {
      return _readResult(
        _api,
        _api.execute(
          _requireOpenHandle(),
          operation,
          keyBuffer.pointer,
          keyBuffer.length,
          valueBuffer.pointer,
          valueBuffer.length,
          setCondition,
          ttlMilliseconds == 0 ? 0 : 1,
          ttlMilliseconds,
        ),
      );
    } finally {
      keyBuffer.close();
      valueBuffer.close();
    }
  }

  _NativeResult _invokeScoped(
    int operation,
    int namespaceId,
    List<int> itemId,
    List<int> value, {
    int flags = 0,
    int ttlMilliseconds = 0,
  }) {
    if (smithyOperationRequiresItemId(operation) &&
        itemId.length != smithyItemIdBytes) {
      throw ArgumentError(
        'itemId must contain exactly $smithyItemIdBytes bytes',
      );
    }
    if (itemId.isNotEmpty && !smithyOperationSupportsScoped(operation)) {
      throw ArgumentError('operation does not accept an itemId');
    }
    final itemBuffer = _Buffer(itemId);
    final valueBuffer = _Buffer(value);
    try {
      return _readResult(
        _api,
        _api.executeScoped(
          _requireOpenHandle(),
          operation,
          namespaceId,
          itemBuffer.pointer,
          itemBuffer.length,
          valueBuffer.pointer,
          valueBuffer.length,
          flags,
          ttlMilliseconds,
        ),
      );
    } finally {
      itemBuffer.close();
      valueBuffer.close();
    }
  }

  NamespaceDescriptor _decodeDescriptor(List<int> payload) {
    final bytes = _Buffer(payload);
    final descriptor = calloc<SmithyNativeDescriptor>();
    try {
      final status = _api.namespaceDescriptorDecode(
        bytes.pointer,
        bytes.length,
        descriptor,
      );
      if (status != smithyDescriptorDecodeOk) {
        throw const OpenKacheClientException(
          'native client returned an invalid namespace descriptor',
        );
      }
      final expiration =
          descriptor.ref.defaultExpiration == smithyDefaultExpirationFixedTtl
          ? ExpirationDefault.fixedTtl
          : ExpirationDefault.noExpiry;
      final eviction =
          descriptor.ref.defaultEviction == smithyDefaultEvictionProtected
          ? EvictionDefault.evictionProtected
          : EvictionDefault.evictable;
      return NamespaceDescriptor(
        namespaceId: descriptor.ref.namespaceId,
        revision: descriptor.ref.revision,
        policy: NamespacePolicy(
          defaultExpiration: expiration,
          defaultTtlMilliseconds: expiration == ExpirationDefault.fixedTtl
              ? descriptor.ref.defaultTtlMs
              : null,
          expirationOverride:
              descriptor.ref.expirationOverride == smithyOverrideAllowed
              ? OverridePolicy.allowed
              : OverridePolicy.disallowed,
          defaultEviction: eviction,
          evictionOverride:
              descriptor.ref.evictionOverride == smithyOverrideAllowed
              ? OverridePolicy.allowed
              : OverridePolicy.disallowed,
        ),
      );
    } finally {
      calloc.free(descriptor);
      bytes.close();
    }
  }

  ffi.Pointer<SmithyNativeClient> _requireOpenHandle() {
    if (_closed || _handle == ffi.nullptr) {
      throw const OpenKacheClientException('OpenKache client is closed');
    }
    return _handle;
  }

}

final class _Buffer {
  _Buffer(List<int> value)
    : length = value.length,
      pointer = value.isEmpty ? ffi.nullptr : calloc<ffi.Uint8>(value.length) {
    if (value.isNotEmpty) {
      pointer.asTypedList(value.length).setAll(0, value);
    }
  }

  final ffi.Pointer<ffi.Uint8> pointer;
  final int length;

  void close() {
    if (pointer != ffi.nullptr) {
      calloc.free(pointer);
    }
  }
}

_NativeResult _readResult(
  SmithyNativeApi api,
  ffi.Pointer<SmithyNativeResult> result, {
  bool takeClient = false,
}) {
  if (result == ffi.nullptr) {
    throw const OpenKacheClientException('native client returned a null result');
  }
  try {
    final kind = api.resultKind(result);
    final length = api.resultDataLength(result);
    if (length < 0 || length > 0x7fffffff) {
      throw const OpenKacheClientException(
        'native client returned an oversized payload',
      );
    }
    final data = api.resultData(result);
    if (length != 0 && data == ffi.nullptr) {
      throw const OpenKacheClientException(
        'native client returned a null payload pointer',
      );
    }
    final payload = length == 0
        ? <int>[]
        : data.asTypedList(length).toList(growable: false);
    final client = takeClient ? api.resultTakeClient(result) : ffi.nullptr;
    if (kind == smithyResultError) {
      throw OpenKacheClientException(
        utf8.decode(payload, allowMalformed: true).isEmpty
            ? 'native client operation failed'
            : utf8.decode(payload, allowMalformed: true),
      );
    }
    return _NativeResult(kind, payload, client);
  } finally {
    api.resultFree(result);
  }
}

final class _NativeResult {
  const _NativeResult(this.kind, this.payload, this.client);

  final int kind;
  final List<int> payload;
  final ffi.Pointer<SmithyNativeClient> client;
}
