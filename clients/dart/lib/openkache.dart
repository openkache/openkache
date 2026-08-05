library;

import 'dart:convert';
import 'dart:ffi' as ffi;

import 'package:ffi/ffi.dart';

import 'generated_local/smithy_api.dart';
import 'generated_local/smithy_contract.dart';
import 'generated_local/smithy_native_api.dart';

/// Failure reported by the shared Rust client-core ABI.
final class EchoClientException implements Exception {
  const EchoClientException(this.message, [this.cause]);

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
final class EchoClient implements OpenKacheClient {
  EchoClient._(this._api, this._handle);

  final SmithyNativeApi _api;
  ffi.Pointer<SmithyNativeClient> _handle;
  bool _closed = false;

  /// Connects to an OpenKache server through the shared native client core.
  factory EchoClient.connect({
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
      throw EchoClientException(
        'failed to load OpenKache native client',
        error,
      );
    }
    if (api.abiVersion() != smithyFfiAbiVersion) {
      throw const EchoClientException(
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
        throw const EchoClientException(
          'native client did not return a connected handle',
        );
      }
      return EchoClient._(api, client);
    } finally {
      addressBuffer.close();
      serverBuffer.close();
      certificateBuffer.close();
      keyBuffer.close();
    }
  }

  @override
  Future<PingOutput> ping(PingInput input) => _run(() {
    final result = _invoke(smithyOperationPing, const <int>[], const <int>[]);
    _requireKind(result, smithyResultOk, 'PING');
    return const PingOutput();
  });

  @override
  Future<EchoOutput> echo(EchoInput input) => _run(() {
    final result = _invoke(
      smithyOperationEcho,
      const <int>[],
      utf8.encode(input.message),
    );
    _requireKind(result, smithyResultValue, 'ECHO');
    try {
      return EchoOutput(
        message: utf8.decode(result.payload, allowMalformed: false),
      );
    } on FormatException catch (error) {
      throw EchoClientException('ECHO response is not valid UTF-8', error);
    }
  });

  /// Sends one message and returns the echoed text.
  Future<String> echoMessage(String message) async =>
      (await echo(EchoInput(message: message))).message;

  @override
  Future<GetOutput> get(GetInput input) => _run(() {
    final result = _invokeScoped(
      smithyOperationGet,
      input.namespaceId,
      input.itemId,
      const <int>[],
    );
    if (result.kind == smithyResultNotFound) {
      return const GetOutput();
    }
    _requireKind(result, smithyResultValue, 'GET');
    return GetOutput(value: result.payload);
  });

  @override
  Future<SetOutput> set(SetInput input) => _run(() {
    final flags = _setFlags(input);
    final result = _invokeScoped(
      smithyOperationSet,
      input.namespaceId,
      input.itemId,
      input.value,
      flags: flags.flags,
      ttlMilliseconds: flags.ttlMilliseconds,
    );
    final outcome = switch (result.kind) {
      smithyResultCreated => SetOutcome.created,
      smithyResultReplaced => SetOutcome.replaced,
      smithyResultNotStored => SetOutcome.notStored,
      _ => throw _unexpectedKind('SET', result.kind),
    };
    return SetOutput(outcome: outcome);
  });

  @override
  Future<DeleteOutput> delete(DeleteInput input) => _run(() {
    final result = _invokeScoped(
      smithyOperationDelete,
      input.namespaceId,
      input.itemId,
      const <int>[],
    );
    return switch (result.kind) {
      smithyResultDeleted => const DeleteOutput(deleted: true),
      smithyResultNotDeleted => const DeleteOutput(deleted: false),
      _ => throw _unexpectedKind('DELETE', result.kind),
    };
  });

  @override
  Future<StatsOutput> stats(StatsInput input) => _run(() {
    final result = _invokeScoped(
      smithyOperationStats,
      input.namespaceId,
      const <int>[],
      const <int>[],
    );
    _requireKind(result, smithyResultValue, 'STATS');
    try {
      return StatsOutput(
        json: utf8.decode(result.payload, allowMalformed: false),
      );
    } on FormatException catch (error) {
      throw EchoClientException('STATS response is not valid UTF-8', error);
    }
  });

  @override
  Future<SyncOutput> sync(SyncInput input) => _run(() {
    final result = _invokeScoped(
      smithyOperationSync,
      input.namespaceId,
      const <int>[],
      const <int>[],
    );
    _requireKind(result, smithyResultOk, 'SYNC');
    return const SyncOutput();
  });

  @override
  Future<NamespaceOpenOutput> namespaceOpen(NamespaceOpenInput input) =>
      _run(() {
        final name = utf8.encode(input.name);
        if (name.length > smithyNamespaceNameMaxBytes) {
          throw const EchoClientException(
            'namespace name exceeds protocol limit',
          );
        }
        final policy = _policyFlags(input.policy, input.createIfMissing);
        final nameBuffer = _Buffer(name);
        try {
          final result = _readResult(
            _api,
            _api.namespaceOpen(
              _requireOpenHandle(),
              nameBuffer.pointer,
              nameBuffer.length,
              input.createIfMissing ? 1 : 0,
              policy.flags,
              policy.ttlMilliseconds,
            ),
          );
          final created = result.kind == smithyResultCreated;
          if (!created && result.kind != smithyResultOk) {
            throw _unexpectedKind('NAMESPACE_OPEN', result.kind);
          }
          return NamespaceOpenOutput(
            descriptor: _decodeDescriptor(result.payload),
            created: created,
          );
        } finally {
          nameBuffer.close();
        }
      });

  @override
  Future<NamespaceUpdatePolicyOutput> namespaceUpdatePolicy(
    NamespaceUpdatePolicyInput input,
  ) => _run(() {
    final policy = _policyFlags(input.policy, true);
    final result = _readResult(
      _api,
      _api.namespaceUpdatePolicy(
        _requireOpenHandle(),
        input.namespaceId,
        input.expectedRevision,
        policy.flags,
        policy.ttlMilliseconds,
      ),
    );
    _requireKind(result, smithyResultValue, 'NAMESPACE_UPDATE_POLICY');
    return NamespaceUpdatePolicyOutput(
      descriptor: _decodeDescriptor(result.payload),
    );
  });

  @override
  Future<NamespaceDeleteOutput> namespaceDelete(NamespaceDeleteInput input) =>
      _run(() {
        final result = _readResult(
          _api,
          _api.namespaceDelete(
            _requireOpenHandle(),
            input.namespaceId,
            input.expectedRevision,
          ),
        );
        _requireKind(result, smithyResultOk, 'NAMESPACE_DELETE');
        return const NamespaceDeleteOutput();
      });

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
    if ((operation == smithyOperationGet ||
            operation == smithyOperationSet ||
            operation == smithyOperationDelete) &&
        itemId.length != smithyItemIdBytes) {
      throw ArgumentError(
        'itemId must contain exactly $smithyItemIdBytes bytes',
      );
    }
    if (itemId.isNotEmpty &&
        operation != smithyOperationGet &&
        operation != smithyOperationSet &&
        operation != smithyOperationDelete) {
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

  _SetFlags _setFlags(SetInput input) {
    var flags = switch (input.condition ?? SetCondition.any) {
      SetCondition.any => smithySetConditionAny,
      SetCondition.ifAbsent => smithySetConditionIfAbsent,
      SetCondition.ifPresent => smithySetConditionIfPresent,
    };
    final expiration =
        input.expirationMode ??
        (input.ttlMilliseconds == null
            ? ExpirationMode.inherit
            : ExpirationMode.explicitTtl);
    switch (expiration) {
      case ExpirationMode.inherit:
        if (input.ttlMilliseconds != null) {
          throw ArgumentError('INHERIT cannot carry a TTL');
        }
        flags |= smithySetInheritExpirationBits;
      case ExpirationMode.noExpiry:
        if (input.ttlMilliseconds != null) {
          throw ArgumentError('NO_EXPIRY cannot carry a TTL');
        }
        flags |= smithySetNoExpiryBits;
      case ExpirationMode.explicitTtl:
        if (input.ttlMilliseconds == null || input.ttlMilliseconds! <= 0) {
          throw ArgumentError('EXPLICIT_TTL requires a positive TTL');
        }
        flags |= smithySetExplicitTtlBits;
    }
    flags |= switch (input.evictionMode ?? EvictionMode.inherit) {
      EvictionMode.inherit => smithySetInheritEvictionBits,
      EvictionMode.evictable => smithySetEvictableBits,
      EvictionMode.evictionProtected => smithySetEvictionProtectedBits,
    };
    if (input.value.length > smithyMaxValueBytes) {
      throw ArgumentError('value exceeds protocol limit');
    }
    return _SetFlags(flags, input.ttlMilliseconds ?? 0);
  }

  _PolicyFlags _policyFlags(NamespacePolicy? policy, bool required) {
    if (required && policy == null) {
      throw ArgumentError('namespace policy is required');
    }
    if (!required && policy != null) {
      throw ArgumentError('namespace policy requires createIfMissing');
    }
    if (policy == null) return const _PolicyFlags(0, 0);
    var flags = switch (policy.defaultExpiration) {
      ExpirationDefault.noExpiry => smithyPolicyNoExpiryBits,
      ExpirationDefault.fixedTtl => smithyPolicyFixedTtlBits,
    };
    final ttl = policy.defaultTtlMilliseconds ?? 0;
    if (policy.defaultExpiration == ExpirationDefault.fixedTtl) {
      if (ttl <= 0) throw ArgumentError('FIXED_TTL requires a positive TTL');
    } else if (ttl != 0) {
      throw ArgumentError('NO_EXPIRY cannot carry a TTL');
    }
    if (policy.expirationOverride == OverridePolicy.allowed) {
      flags |= smithyPolicyExpirationOverrideFlag;
    }
    if (policy.defaultEviction == EvictionDefault.evictionProtected) {
      flags |= smithyPolicyEvictionProtectedFlag;
    }
    if (policy.evictionOverride == OverridePolicy.allowed) {
      flags |= smithyPolicyEvictionOverrideFlag;
    }
    return _PolicyFlags(flags, ttl);
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
        throw const EchoClientException(
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
      throw const EchoClientException('OpenKache client is closed');
    }
    return _handle;
  }

  static void _requireKind(
    _NativeResult result,
    int expected,
    String operation,
  ) {
    if (result.kind != expected) {
      throw _unexpectedKind(operation, result.kind);
    }
  }

  static EchoClientException _unexpectedKind(String operation, int kind) =>
      EchoClientException('$operation returned unexpected native result $kind');
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
    throw const EchoClientException('native client returned a null result');
  }
  try {
    final kind = api.resultKind(result);
    final length = api.resultDataLength(result);
    if (length < 0 || length > 0x7fffffff) {
      throw const EchoClientException(
        'native client returned an oversized payload',
      );
    }
    final data = api.resultData(result);
    if (length != 0 && data == ffi.nullptr) {
      throw const EchoClientException(
        'native client returned a null payload pointer',
      );
    }
    final payload = length == 0
        ? <int>[]
        : data.asTypedList(length).toList(growable: false);
    final client = takeClient ? api.resultTakeClient(result) : ffi.nullptr;
    if (kind == smithyResultError) {
      throw EchoClientException(
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

final class _SetFlags {
  const _SetFlags(this.flags, this.ttlMilliseconds);

  final int flags;
  final int ttlMilliseconds;
}

final class _PolicyFlags {
  const _PolicyFlags(this.flags, this.ttlMilliseconds);

  final int flags;
  final int ttlMilliseconds;
}
