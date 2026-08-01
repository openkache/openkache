/// Async Dart binding over the OpenKache shared native ABI.
library;

import 'dart:async';
import 'dart:convert';
import 'dart:ffi' as ffi;
import 'dart:io';
import 'dart:isolate';
import 'dart:math';
import 'dart:typed_data';

import 'package:ffi/ffi.dart';

import 'generated_contract.dart' as smithy;

const int _resultError = 0;
const int _resultOk = 1;
const int _resultValue = 2;
const int _resultNotFound = 3;
const int _resultCreated = 4;
const int _resultReplaced = 5;
const int _resultDeleted = 6;
const int _resultNotDeleted = 7;
const int _resultConnected = 8;
const int _resultNotStored = 9;
const int _opcodePing = smithy.smithy_opcode_ping;
const int _opcodeGet = smithy.smithy_opcode_get;
const int _opcodeSet = smithy.smithy_opcode_set;
const int _opcodeDelete = smithy.smithy_opcode_delete;
const int _opcodeStats = smithy.smithy_opcode_stats;
const int _opcodeSync = smithy.smithy_opcode_sync;
const int _operationGetJson = smithy.smithy_ffi_operation_get_json;
const int _operationSetJson = smithy.smithy_ffi_operation_set_json;
const int _operationReconnect = smithy.smithy_ffi_operation_reconnect;
const int _conditionNone = smithy.smithy_ffi_set_condition_none;
const int _conditionIfAbsent = smithy.smithy_ffi_set_condition_if_absent;
const int _conditionIfPresent = smithy.smithy_ffi_set_condition_if_present;
const int _mutationIdBytes = smithy.smithyMutationIdBytes;
const int _itemIdBytes = smithy.smithyItemIdBytes;
const int _keyBytes = smithy.smithyValueDataProtectionKeyBytes;
const int _maxPreviousKeys = 8;
const int _ffiErrorCancelled = smithy.smithy_ffi_error_cancelled;

typedef _ConnectOptionsNative = ffi.Pointer<ffi.Void> Function(
  ffi.Pointer<_ConnectOptions>,
);
typedef _AbiVersionNative = ffi.Uint32 Function();

typedef _ExecuteRequestNative = ffi.Pointer<ffi.Void> Function(
  ffi.Pointer<ffi.Void>,
  ffi.Uint64,
  ffi.Uint32,
  ffi.Pointer<ffi.Uint8>,
  ffi.IntPtr,
  ffi.Pointer<ffi.Uint8>,
  ffi.IntPtr,
  ffi.Uint32,
  ffi.Uint8,
  ffi.Uint64,
);

typedef _ExecuteMutationRequestNative = ffi.Pointer<ffi.Void> Function(
  ffi.Pointer<ffi.Void>,
  ffi.Uint64,
  ffi.Uint32,
  ffi.Pointer<ffi.Uint8>,
  ffi.IntPtr,
  ffi.Pointer<ffi.Uint8>,
  ffi.IntPtr,
  ffi.Uint32,
  ffi.Uint8,
  ffi.Uint64,
  ffi.Pointer<ffi.Uint8>,
  ffi.IntPtr,
);

typedef _ResultKindNative = ffi.Uint32 Function(ffi.Pointer<ffi.Void>);
typedef _ResultDataNative = ffi.Pointer<ffi.Uint8> Function(ffi.Pointer<ffi.Void>);
typedef _ResultLengthNative = ffi.IntPtr Function(ffi.Pointer<ffi.Void>);
typedef _ResultMetadataNative = ffi.Uint8 Function(
  ffi.Pointer<ffi.Void>,
  ffi.Pointer<_ErrorMetadata>,
);
typedef _TakeClientNative = ffi.Pointer<ffi.Void> Function(ffi.Pointer<ffi.Void>);
typedef _FreeNative = ffi.Void Function(ffi.Pointer<ffi.Void>);
typedef _CancelNative = ffi.Uint8 Function(ffi.Pointer<ffi.Void>, ffi.Uint64);
typedef _MetricsNative = ffi.Uint8 Function(
  ffi.Pointer<ffi.Void>,
  ffi.Pointer<_MetricsSnapshot>,
);
typedef _ConnectionStateNative = ffi.Uint32 Function(ffi.Pointer<ffi.Void>);

final class _ConnectOptions extends ffi.Struct {
  external ffi.Pointer<ffi.Uint8> address;
  @ffi.IntPtr()
  external int addressLength;
  external ffi.Pointer<ffi.Uint8> serverName;
  @ffi.IntPtr()
  external int serverNameLength;
  external ffi.Pointer<ffi.Uint8> certificate;
  @ffi.IntPtr()
  external int certificateLength;
  external ffi.Pointer<ffi.Uint8> clientCertificateChain;
  @ffi.IntPtr()
  external int clientCertificateChainLength;
  external ffi.Pointer<ffi.Uint8> clientPrivateKey;
  @ffi.IntPtr()
  external int clientPrivateKeyLength;
  external ffi.Pointer<ffi.Uint8> dataProtectionKey;
  @ffi.IntPtr()
  external int dataProtectionKeyLength;
  external ffi.Pointer<ffi.Uint8> previousDataProtectionKeys;
  @ffi.IntPtr()
  external int previousDataProtectionKeysLength;
  @ffi.IntPtr()
  external int previousDataProtectionKeyCount;
  @ffi.Uint8()
  external int compressionEnabled;
  @ffi.Int32()
  external int compressionLevel;
  @ffi.IntPtr()
  external int minimumInputSize;
  @ffi.IntPtr()
  external int minimumSavings;
  @ffi.Uint32()
  external int encryption;
  @ffi.Uint64()
  external int connectTimeoutMs;
  @ffi.Uint64()
  external int requestTimeoutMs;
  @ffi.IntPtr()
  external int retryMaxAttempts;
  @ffi.IntPtr()
  external int maxInFlight;
}

final class _ErrorMetadata extends ffi.Struct {
  @ffi.Uint32()
  external int code;
  @ffi.Uint32()
  external int operation;
  @ffi.Uint32()
  external int phase;
  @ffi.Uint32()
  external int backend;
  @ffi.Uint8()
  external int retryable;
  @ffi.Uint8()
  external int ambiguous;
  @ffi.Uint8()
  external int mutationIdLength;
  @ffi.Uint8()
  external int reserved;
  @ffi.Array(_mutationIdBytes)
  external ffi.Array<ffi.Uint8> mutationId;
}

final class _MetricsSnapshot extends ffi.Struct {
  @ffi.Uint64()
  external int requests;
  @ffi.Uint64()
  external int hits;
  @ffi.Uint64()
  external int misses;
  @ffi.Uint64()
  external int retries;
  @ffi.Uint64()
  external int reconnects;
  @ffi.Uint64()
  external int cancellations;
  @ffi.Uint64()
  external int transportErrors;
  @ffi.Uint64()
  external int protocolErrors;
  @ffi.Uint64()
  external int bytesSent;
  @ffi.Uint64()
  external int bytesReceived;
  @ffi.Uint64()
  external int activeLanes;
}

/// Atomic SET/DELETE condition values from the shared Smithy contract.
enum SetCondition {
  none(_conditionNone),
  ifAbsent(_conditionIfAbsent),
  ifPresent(_conditionIfPresent);

  const SetCondition(this.code);

  final int code;
}

/// Structured metadata attached to a native operation failure.
class ErrorMetadata {
  const ErrorMetadata({
    required this.code,
    required this.operation,
    required this.phase,
    required this.backend,
    required this.retryable,
    required this.ambiguous,
    this.mutationId,
  });

  final int code;
  final int operation;
  final int phase;
  final int backend;
  final bool retryable;
  final bool ambiguous;
  final Uint8List? mutationId;
}

/// Native operation failure with retry and ambiguity metadata.
class OpenKacheError implements Exception {
  OpenKacheError(this.message, {this.metadata});

  final String message;
  final ErrorMetadata? metadata;

  bool get retryable => metadata?.retryable ?? false;
  bool get ambiguous => metadata?.ambiguous ?? false;
  bool get cancelled => metadata?.code == _ffiErrorCancelled;

  @override
  String toString() => 'OpenKacheError: $message';
}

/// Point-in-time native request and transport counters.
class MetricsSnapshot {
  const MetricsSnapshot({
    required this.requests,
    required this.hits,
    required this.misses,
    required this.retries,
    required this.reconnects,
    required this.cancellations,
    required this.transportErrors,
    required this.protocolErrors,
    required this.bytesSent,
    required this.bytesReceived,
    required this.activeLanes,
  });

  final int requests;
  final int hits;
  final int misses;
  final int retries;
  final int reconnects;
  final int cancellations;
  final int transportErrors;
  final int protocolErrors;
  final int bytesSent;
  final int bytesReceived;
  final int activeLanes;
}

/// A caller-owned cancellation source for one or more requests.
class CancellationToken {
  bool _cancelled = false;
  final List<void Function()> _callbacks = <void Function()>[];

  bool get isCancelled => _cancelled;

  /// Cancels all requests currently attached to this token.
  void cancel() {
    if (_cancelled) return;
    _cancelled = true;
    final callbacks = List<void Function()>.from(_callbacks);
    _callbacks.clear();
    for (final callback in callbacks) {
      callback();
    }
  }

  void _register(void Function() callback) {
    if (_cancelled) {
      callback();
    } else {
      _callbacks.add(callback);
    }
  }
}

/// A Future plus an explicit native cancellation operation.
class OpenKacheRequest<T> {
  OpenKacheRequest(this.future, this._cancelCallback);

  final Future<T> future;
  final bool Function() _cancelCallback;

  /// Cancels the native request and returns whether it was still pending.
  bool cancel() => _cancelCallback();
}

/// Idempotency and TTL options for SET and DELETE.
class SetOptions {
  const SetOptions({
    this.condition = SetCondition.none,
    this.ttl,
    this.mutationId,
  });

  final SetCondition condition;
  final Duration? ttl;
  final Uint8List? mutationId;

  int get ttlMilliseconds {
    if (ttl == null) return 0;
    final milliseconds = ttl!.inMilliseconds;
    if (milliseconds <= 0) {
      throw ArgumentError.value(ttl, 'ttl', 'must be positive');
    }
    return milliseconds;
  }

  Uint8List? validateMutationId() {
    if (mutationId == null) return null;
    if (mutationId!.length != _mutationIdBytes) {
      throw ArgumentError.value(
        mutationId,
        'mutationId',
        'must contain exactly $_mutationIdBytes bytes',
      );
    }
    return Uint8List.fromList(mutationId!);
  }

  SetOptions withMutationId(Uint8List value) => SetOptions(
        condition: condition,
        ttl: ttl,
        mutationId: Uint8List.fromList(value),
      );
}

/// Active data-protection key and up to eight retired keys.
class DataProtectionKeyRing {
  DataProtectionKeyRing({
    required Uint8List active,
    List<Uint8List> previous = const <Uint8List>[],
  })  : active = Uint8List.fromList(active),
        previous = previous
            .map((key) => Uint8List.fromList(key))
            .toList(growable: false) {
    _validateKey(this.active, 'keyRing.active');
    if (this.previous.length > _maxPreviousKeys) {
      throw ArgumentError.value(
        previous,
        'keyRing.previous',
        'may contain at most $_maxPreviousKeys keys',
      );
    }
    for (final key in this.previous) {
      _validateKey(key, 'keyRing.previous entry');
    }
  }

  final Uint8List active;
  final List<Uint8List> previous;
}

/// Dart client backed by the Rust worker and C ABI.
class OpenKacheClient {
  OpenKacheClient._(
    this._library,
    this._nativePath,
    this._client,
    this._free,
  );

  final ffi.DynamicLibrary _library;
  final String _nativePath;
  ffi.Pointer<ffi.Void> _client;
  final _FreeNative _free;
  int _nextRequestId = 1;
  bool _closed = false;
  bool _closing = false;
  Future<void>? _closeFuture;
  final Map<int, Future<void>> _activeRequests = <int, Future<void>>{};
  final Map<int, bool Function()> _requestCancels =
      <int, bool Function()>{};

  /// Connects asynchronously to one OpenKache endpoint.
  static Future<OpenKacheClient> connect({
    required String address,
    required Uint8List certificate,
    Uint8List? dataProtectionKey,
    DataProtectionKeyRing? keyRing,
    String serverName = 'localhost',
    List<Uint8List> previousDataProtectionKeys = const <Uint8List>[],
    bool compressionEnabled = false,
    int compressionLevel = smithy.smithyDefaultZstandardLevel,
    int minimumInputBytes = smithy.smithyDefaultZstandardMinimumInputBytes,
    int minimumSavings = smithy.smithyDefaultZstandardMinimumSavingsBytes,
    int encryption = 2,
    int retryMaxAttempts = 2,
    int maxInFlight = 256,
    Duration connectTimeout = const Duration(
      milliseconds: smithy.smithyDefaultConnectTimeoutMilliseconds,
    ),
    Duration requestTimeout = const Duration(
      milliseconds: smithy.smithyDefaultRequestTimeoutMilliseconds,
    ),
    String? nativePath,
  }) async {
    if (keyRing != null &&
        (dataProtectionKey != null || previousDataProtectionKeys.isNotEmpty)) {
      throw ArgumentError(
        'keyRing cannot be combined with dataProtectionKey or previousDataProtectionKeys',
      );
    }
    final activeKey = keyRing?.active ?? dataProtectionKey;
    if (activeKey == null) {
      throw ArgumentError('dataProtectionKey or keyRing must be supplied');
    }
    _validateKey(activeKey, 'dataProtectionKey');
    final retiredKeys = keyRing?.previous ?? previousDataProtectionKeys;
    if (retiredKeys.length > _maxPreviousKeys) {
      throw ArgumentError.value(
        retiredKeys,
        'previousDataProtectionKeys',
        'may contain at most $_maxPreviousKeys keys',
      );
    }
    for (final key in retiredKeys) {
      _validateKey(key, 'previousDataProtectionKeys entry');
    }
    if (connectTimeout <= Duration.zero || requestTimeout <= Duration.zero) {
      throw ArgumentError('connection and request timeouts must be positive');
    }
    if (retryMaxAttempts <= 0 || maxInFlight <= 0) {
      throw ArgumentError('retryMaxAttempts and maxInFlight must be positive');
    }
    final resolvedPath = nativePath ??
        Platform.environment['OPENKACHE_CLIENT_NATIVE'] ??
        _defaultNativePath();
    return Future<OpenKacheClient>(() {
      final library = ffi.DynamicLibrary.open(resolvedPath);
      final connect = library.lookupFunction<
          _ConnectOptionsNative, _ConnectOptionsNative>(
        'openkache_client_connect_with_options',
      );
      final abiVersion = library.lookupFunction<_AbiVersionNative, _AbiVersionNative>(
        'openkache_client_abi_version',
      );
      final nativeAbiVersion = abiVersion();
      if (nativeAbiVersion != smithy.smithyFfiAbiVersion) {
        throw OpenKacheError(
          'unsupported native client ABI version $nativeAbiVersion',
        );
      }
      final resultKind = library.lookupFunction<_ResultKindNative, _ResultKindNative>(
        'openkache_client_result_kind',
      );
      final takeClient = library.lookupFunction<_TakeClientNative, _TakeClientNative>(
        'openkache_client_result_take_client',
      );
      final freeResult = library.lookupFunction<_FreeNative, _FreeNative>(
        'openkache_client_result_free',
      );
    final buffers = <_NativeBytes>[];
      ffi.Pointer<_ConnectOptions>? optionsPointer;
      try {
        final addressBuffer = _nativeBytes(address, buffers);
        final serverNameBuffer = _nativeBytes(serverName, buffers);
        final certificateBuffer = _nativeBytes(certificate, buffers);
        final keyBuffer = _nativeBytes(activeKey, buffers);
        final previousBytes = Uint8List.fromList(
          retiredKeys.expand((key) => key).toList(),
        );
        final previousBuffer = _nativeBytes(previousBytes, buffers);
        optionsPointer = calloc<_ConnectOptions>();
        final options = optionsPointer.ref;
        options
          ..address = addressBuffer.pointer
          ..addressLength = addressBuffer.length
          ..serverName = serverNameBuffer.pointer
          ..serverNameLength = serverNameBuffer.length
          ..certificate = certificateBuffer.pointer
          ..certificateLength = certificateBuffer.length
          ..clientCertificateChain = ffi.nullptr
          ..clientCertificateChainLength = 0
          ..clientPrivateKey = ffi.nullptr
          ..clientPrivateKeyLength = 0
          ..dataProtectionKey = keyBuffer.pointer
          ..dataProtectionKeyLength = keyBuffer.length
          ..previousDataProtectionKeys = previousBuffer.pointer
          ..previousDataProtectionKeysLength = previousBuffer.length
          ..previousDataProtectionKeyCount = retiredKeys.length
          ..compressionEnabled = compressionEnabled ? 1 : 0
          ..compressionLevel = compressionLevel
          ..minimumInputSize = minimumInputBytes
          ..minimumSavings = minimumSavings
          ..encryption = encryption
          ..connectTimeoutMs = connectTimeout.inMilliseconds
          ..requestTimeoutMs = requestTimeout.inMilliseconds
          ..retryMaxAttempts = retryMaxAttempts
          ..maxInFlight = maxInFlight;
        final result = connect(optionsPointer);
        if (result == ffi.nullptr) {
          throw OpenKacheError('native connect returned a null result');
        }
        final kind = resultKind(result);
        if (kind != _resultConnected) {
          final failure = _readResult(library, result);
          throw OpenKacheError(failure.message, metadata: failure.metadata);
        }
        final client = takeClient(result);
        freeResult(result);
        if (client == ffi.nullptr) {
          throw OpenKacheError('native connect returned no client');
        }
        return OpenKacheClient._(
          library,
          resolvedPath,
          client,
          library.lookupFunction<_FreeNative, _FreeNative>(
            'openkache_client_free',
          ),
        );
      } finally {
        if (optionsPointer != null) calloc.free(optionsPointer);
        for (final buffer in buffers) {
          _zeroizeNativeBytes(buffer);
          calloc.free(buffer.pointer);
        }
      }
    });
  }

  /// Gets protected bytes, or null when absent.
  Future<Uint8List?> get(
    Uint8List key, {
    CancellationToken? cancellationToken,
  }) =>
      getRequest(key, cancellationToken: cancellationToken).future;

  /// Starts a protected byte GET and exposes native cancellation.
  OpenKacheRequest<Uint8List?> getRequest(
    Uint8List key, {
    CancellationToken? cancellationToken,
  }) =>
      _mapRequest(
        _execute(_opcodeGet, key, Uint8List(0), SetOptions(), false,
            cancellationToken),
        (result) {
          if (result.kind == _resultNotFound) return null;
          return _requireValue(result, 'GET');
        },
      );

  /// Retrieves a canonical JSON value, or null when absent.
  Future<Object?> getJson(
    Uint8List key, {
    CancellationToken? cancellationToken,
  }) async {
    final result = await _mapRequest(
      _execute(_operationGetJson, key, Uint8List(0), SetOptions(), false,
          cancellationToken),
      (value) {
        if (value.kind == _resultNotFound) return null;
        final bytes = _requireValue(value, 'GET_JSON');
        return jsonDecode(utf8.decode(bytes));
      },
    ).future;
    return result;
  }

  /// Stores protected bytes.
  Future<String> set(
    Uint8List key,
    Uint8List value, [
    SetOptions options = const SetOptions(),
  ], {
    CancellationToken? cancellationToken,
  }) async {
    final result = await setRequest(
      key,
      value,
      options: options,
      cancellationToken: cancellationToken,
    ).future;
    return result;
  }

  /// Starts a protected byte SET and exposes native cancellation.
  OpenKacheRequest<String> setRequest(
    Uint8List key,
    Uint8List value, {
    SetOptions options = const SetOptions(),
    CancellationToken? cancellationToken,
  }) =>
      _mapRequest(
        _execute(
          _opcodeSet,
          key,
          value,
          _mutationOptions(options),
          false,
          cancellationToken,
        ),
        (result) => _setOutcome(result, 'SET'),
      );

  /// Stores a canonical JSON value using the core JSON representation.
  Future<String> setJson(
    Uint8List key,
    Object? value, [
    SetOptions options = const SetOptions(),
  ], {
    CancellationToken? cancellationToken,
  ]) async {
    final result = await _mapRequest(
      _execute(
        _operationSetJson,
        key,
        Uint8List.fromList(utf8.encode(jsonEncode(value))),
        _mutationOptions(options),
        false,
        cancellationToken,
      ),
      (value) => _setOutcome(value, 'SET_JSON'),
    ).future;
    return result;
  }

  /// Deletes protected bytes.
  Future<bool> delete(
    Uint8List key, [
    SetOptions options = const SetOptions(),
  ], {
    CancellationToken? cancellationToken,
  }) async {
    final result = await deleteRequest(
      key,
      options: options,
      cancellationToken: cancellationToken,
    ).future;
    return result;
  }

  /// Starts a protected DELETE and exposes native cancellation.
  OpenKacheRequest<bool> deleteRequest(
    Uint8List key, {
    SetOptions options = const SetOptions(),
    CancellationToken? cancellationToken,
  }) =>
      _mapRequest(
        _execute(
          _opcodeDelete,
          key,
          Uint8List(0),
          _mutationOptions(options),
          false,
          cancellationToken,
        ),
        (result) {
          switch (result.kind) {
            case _resultDeleted:
              return true;
            case _resultNotDeleted:
            case _resultNotFound:
              return false;
            default:
              throw OpenKacheError('unexpected DELETE result ${result.kind}');
          }
        },
      );

  /// Gets exact bytes for a 32-byte protocol item ID.
  Future<Uint8List?> getRaw(
    Uint8List itemId, {
    CancellationToken? cancellationToken,
  }) =>
      getRawRequest(itemId, cancellationToken: cancellationToken).future;

  /// Starts a raw GET and exposes native cancellation.
  OpenKacheRequest<Uint8List?> getRawRequest(
    Uint8List itemId, {
    CancellationToken? cancellationToken,
  }) {
    _validateItemId(itemId);
    return _mapRequest(
      _execute(_opcodeGet, itemId, Uint8List(0), SetOptions(), true,
          cancellationToken),
      (result) {
        if (result.kind == _resultNotFound) return null;
        return _requireValue(result, 'RAW_GET');
      },
    );
  }

  /// Stores exact bytes for a 32-byte protocol item ID.
  Future<String> setRaw(
    Uint8List itemId,
    Uint8List value, [
    SetOptions options = const SetOptions(),
  ], {
    CancellationToken? cancellationToken,
  ]) async {
    _validateItemId(itemId);
    return _mapRequest(
      _execute(
        _opcodeSet,
        itemId,
        value,
        _mutationOptions(options),
        true,
        cancellationToken,
      ),
      (result) => _setOutcome(result, 'RAW_SET'),
    ).future;
  }

  /// Deletes an exact 32-byte protocol item ID.
  Future<bool> deleteRaw(
    Uint8List itemId, [
    SetOptions options = const SetOptions(),
  ], {
    CancellationToken? cancellationToken,
  ]) async {
    _validateItemId(itemId);
    return _mapRequest(
      _execute(
        _opcodeDelete,
        itemId,
        Uint8List(0),
        _mutationOptions(options),
        true,
        cancellationToken,
      ),
      (result) {
        switch (result.kind) {
          case _resultDeleted:
            return true;
          case _resultNotDeleted:
          case _resultNotFound:
            return false;
          default:
            throw OpenKacheError('unexpected RAW_DELETE result ${result.kind}');
        }
      },
    ).future;
  }

  /// Returns server statistics as UTF-8 JSON.
  Future<String> stats({CancellationToken? cancellationToken}) async {
    final result = await _mapRequest(
      _execute(_opcodeStats, Uint8List(0), Uint8List(0), SetOptions(), false,
          cancellationToken),
      (value) => utf8.decode(_requireValue(value, 'STATS')),
    ).future;
    return result;
  }

  /// Waits for a server durability barrier.
  Future<void> sync({CancellationToken? cancellationToken}) async {
    final result = await _execute(
      _opcodeSync,
      Uint8List(0),
      Uint8List(0),
      SetOptions(),
      false,
      cancellationToken,
    ).future;
    if (result.kind != _resultOk) {
      throw OpenKacheError('unexpected SYNC result ${result.kind}');
    }
  }

  /// Reconnects without replaying an operation.
  Future<void> reconnect({CancellationToken? cancellationToken}) async {
    final result = await _execute(
      _operationReconnect,
      Uint8List(0),
      Uint8List(0),
      SetOptions(),
      false,
      cancellationToken,
    ).future;
    if (result.kind != _resultOk) {
      throw OpenKacheError('unexpected RECONNECT result ${result.kind}');
    }
  }

  /// Returns the shared core's best-effort connection state.
  int get connectionState {
    if (_closed) return 3;
    final state = _library.lookupFunction<_ConnectionStateNative, _ConnectionStateNative>(
      'openkache_client_connection_state',
    );
    return state(_client);
  }

  /// Returns a point-in-time native metrics snapshot.
  MetricsSnapshot metricsSnapshot() {
    if (_closed) throw StateError('client is closed');
    final metrics = _library.lookupFunction<_MetricsNative, _MetricsNative>(
      'openkache_client_metrics_snapshot',
    );
    final pointer = calloc<_MetricsSnapshot>();
    try {
      if (metrics(_client, pointer) == 0) {
        throw OpenKacheError('native metrics snapshot failed');
      }
      final value = pointer.ref;
      return MetricsSnapshot(
        requests: value.requests,
        hits: value.hits,
        misses: value.misses,
        retries: value.retries,
        reconnects: value.reconnects,
        cancellations: value.cancellations,
        transportErrors: value.transportErrors,
        protocolErrors: value.protocolErrors,
        bytesSent: value.bytesSent,
        bytesReceived: value.bytesReceived,
        activeLanes: value.activeLanes,
      );
    } finally {
      calloc.free(pointer);
    }
  }

  /// Closes the native worker after all isolate FFI calls have returned.
  ///
  /// The returned future must be awaited before the client handle is reused or
  /// discarded by the caller. This keeps the native pointer alive while an
  /// isolate may still be inside the FFI call.
  Future<void> close() {
    final existing = _closeFuture;
    if (existing != null) return existing;
    if (_closed) return Future<void>.value();
    _closing = true;
    _closed = true;
    _closeFuture = _finishClose();
    return _closeFuture!;
  }

  Future<void> _finishClose() async {
    for (final cancel in List<bool Function()>.from(_requestCancels.values)) {
      cancel();
    }
    final pending = List<Future<void>>.from(_activeRequests.values);
    if (pending.isNotEmpty) {
      await Future.wait<void>(pending);
    }
    _free(_client);
    _client = ffi.nullptr;
    _closing = false;
  }

  OpenKacheRequest<_Result> _execute(
    int operation,
    Uint8List key,
    Uint8List value,
    SetOptions options,
    bool raw,
    CancellationToken? cancellationToken,
  ) {
    if (_closed || _closing) {
      throw StateError('client is closed');
    }
    final requestId = _allocateRequestId();
    final completer = Completer<_Result>();
    var started = false;
    var cancelRequested = false;
    var complete = false;
    final request = OpenKacheRequest<_Result>(
      completer.future,
      () {
        if (complete || cancelRequested) return false;
        cancelRequested = true;
        final nativeFound = _cancel(requestId);
        if (!nativeFound || !started) {
          if (!completer.isCompleted) {
            completer.completeError(
              OpenKacheError(
                'client operation canceled',
                metadata: const ErrorMetadata(
                  code: _ffiErrorCancelled,
                  operation: 0,
                  phase: 0,
                  backend: 0,
                  retryable: false,
                  ambiguous: false,
                ),
              ),
            );
          }
        }
        return true;
      },
    );
    final execution = Future<void>(() async {
      try {
        if (cancelRequested) return;
        started = true;
        final encoded = await _runNativeRequest(
          _nativePath,
          _client.address,
          requestId,
          operation,
          key,
          value,
          options.condition.code,
          options.ttl == null ? 0 : 1,
          options.ttlMilliseconds,
          options.validateMutationId() ?? Uint8List(0),
          raw,
        );
        if (complete || completer.isCompleted) return;
        if (cancelRequested) {
          complete = true;
          completer.completeError(
            OpenKacheError(
              'client operation canceled',
              metadata: const ErrorMetadata(
                code: _ffiErrorCancelled,
                operation: 0,
                phase: 0,
                backend: 0,
                retryable: false,
                ambiguous: false,
              ),
            ),
          );
          return;
        }
        final metadata = encoded[2] is List
            ? _metadataFromList(encoded[2]! as List<Object?>)
            : null;
        final result = _Result(
          encoded[0]! as int,
          Uint8List.fromList(encoded[1]! as List<int>),
          metadata,
        );
        if (result.kind == _resultError) {
          throw OpenKacheError(
            utf8.decode(result.payload),
            metadata: result.metadata,
          );
        }
        complete = true;
        completer.complete(result);
      } catch (error, stack) {
        if (!completer.isCompleted) {
          complete = true;
          completer.completeError(error, stack);
        }
      } finally {
        _activeRequests.remove(requestId);
        _requestCancels.remove(requestId);
      }
    });
    _activeRequests[requestId] = execution;
    _requestCancels[requestId] = request.cancel;
    cancellationToken?._register(request.cancel);
    return request;
  }

  int _allocateRequestId() {
    while (true) {
      final requestId = _nextRequestId;
      _nextRequestId++;
      if (_nextRequestId == 0x7fff_ffff_ffff_ffff) {
        _nextRequestId = 1;
      }
      if (!_activeRequests.containsKey(requestId)) return requestId;
    }
  }

  OpenKacheRequest<T> _mapRequest<T>(
    OpenKacheRequest<_Result> source,
    T Function(_Result) mapper,
  ) {
    final completer = Completer<T>();
    source.future.then(
      (result) {
        try {
          completer.complete(mapper(result));
        } catch (error, stack) {
          completer.completeError(error, stack);
        }
      },
      onError: (Object error, StackTrace stack) {
        if (!completer.isCompleted) completer.completeError(error, stack);
      },
    );
    return OpenKacheRequest<T>(completer.future, source.cancel);
  }

  bool _cancel(int requestId) {
    if (_closed && !_closing) return false;
    final cancel = _library.lookupFunction<_CancelNative, _CancelNative>(
      'openkache_client_cancel',
    );
    return cancel(_client, requestId) != 0;
  }
}

class _Result {
  const _Result(this.kind, this.payload, this.metadata);

  final int kind;
  final Uint8List payload;
  final ErrorMetadata? metadata;
}

class _NativeBytes {
  const _NativeBytes(this.pointer, this.length);

  final ffi.Pointer<ffi.Uint8> pointer;
  final int length;
}

_NativeBytes _nativeBytes(Object value, List<_NativeBytes> owned) {
  final bytes = value is String
      ? Uint8List.fromList(utf8.encode(value))
      : value as Uint8List;
  if (bytes.isEmpty) return const _NativeBytes(ffi.nullptr, 0);
  final pointer = calloc<ffi.Uint8>(bytes.length);
  pointer.asTypedList(bytes.length).setAll(0, bytes);
  final nativeBytes = _NativeBytes(pointer, bytes.length);
  owned.add(nativeBytes);
  return nativeBytes;
}

void _zeroizeNativeBytes(_NativeBytes value) {
  if (value.length != 0) {
    value.pointer.asTypedList(value.length).fillRange(0, value.length, 0);
  }
}

String _defaultNativePath() {
  if (Platform.isLinux) return 'libopenkache_client_core.so';
  if (Platform.isMacOS) return 'libopenkache_client_core.dylib';
  if (Platform.isWindows) return 'openkache_client_core.dll';
  throw UnsupportedError('unsupported platform for OpenKache native client');
}

void _validateKey(Uint8List key, String name) {
  if (key.length != _keyBytes) {
    throw ArgumentError.value(key, name, 'must contain exactly $_keyBytes bytes');
  }
}

void _validateItemId(Uint8List itemId) {
  if (itemId.length != _itemIdBytes) {
    throw ArgumentError.value(
      itemId,
      'itemId',
      'must contain exactly $_itemIdBytes bytes',
    );
  }
}

SetOptions _mutationOptions(SetOptions options) {
  final mutationId = options.validateMutationId();
  if (mutationId != null) return options;
  final generated = Uint8List(_mutationIdBytes);
  final random = Random.secure();
  for (var index = 0; index < generated.length; index++) {
    generated[index] = random.nextInt(256);
  }
  return options.withMutationId(generated);
}

Uint8List _requireValue(_Result result, String operation) {
  if (result.kind != _resultValue) {
    throw OpenKacheError('unexpected $operation result ${result.kind}');
  }
  return result.payload;
}

String _setOutcome(_Result result, String operation) {
  switch (result.kind) {
    case _resultCreated:
      return 'created';
    case _resultReplaced:
      return 'replaced';
    case _resultNotStored:
      return 'not_stored';
    default:
      throw OpenKacheError('unexpected $operation result ${result.kind}');
  }
}

class _ReadResult {
  const _ReadResult(this.message, this.metadata);

  final String message;
  final ErrorMetadata? metadata;
}

_ReadResult _readResult(ffi.DynamicLibrary library, ffi.Pointer<ffi.Void> result) {
  final data = library.lookupFunction<_ResultDataNative, _ResultDataNative>(
    'openkache_client_result_data',
  );
  final length = library.lookupFunction<_ResultLengthNative, _ResultLengthNative>(
    'openkache_client_result_data_length',
  );
  final metadata = library.lookupFunction<_ResultMetadataNative, _ResultMetadataNative>(
    'openkache_client_result_error_metadata',
  );
  final free = library.lookupFunction<_FreeNative, _FreeNative>(
    'openkache_client_result_free',
  );
  final payloadLength = length(result);
  final payload = payloadLength == 0
      ? Uint8List(0)
      : Uint8List.fromList(data(result).asTypedList(payloadLength));
  final metadataPointer = calloc<_ErrorMetadata>();
  ErrorMetadata? errorMetadata;
  try {
    if (metadata(result, metadataPointer) != 0) {
      errorMetadata = ErrorMetadata(
        code: metadataPointer.ref.code,
        operation: metadataPointer.ref.operation,
        phase: metadataPointer.ref.phase,
        backend: metadataPointer.ref.backend,
        retryable: metadataPointer.ref.retryable != 0,
        ambiguous: metadataPointer.ref.ambiguous != 0,
        mutationId: _readMutationId(
          metadataPointer.ref.mutationId,
          metadataPointer.ref.mutationIdLength,
        ),
      );
    }
  } finally {
    calloc.free(metadataPointer);
    free(result);
  }
  return _ReadResult(utf8.decode(payload), errorMetadata);
}

Future<List<Object?>> _runNativeRequest(
  String nativePath,
  int clientAddress,
  int requestId,
  int operation,
  Uint8List key,
  Uint8List value,
  int condition,
  int ttlEnabled,
  int ttlMilliseconds,
  Uint8List mutationId,
  bool raw,
) {
  return Isolate.run<List<Object?>>(
    () => _runNativeRequestSync(
      nativePath,
      clientAddress,
      requestId,
      operation,
      key,
      value,
      condition,
      ttlEnabled,
      ttlMilliseconds,
      mutationId,
      raw,
    ),
  );
}

List<Object?> _runNativeRequestSync(
  String nativePath,
  int clientAddress,
  int requestId,
  int operation,
  Uint8List key,
  Uint8List value,
  int condition,
  int ttlEnabled,
  int ttlMilliseconds,
  Uint8List mutationId,
  bool raw,
) {
  final library = ffi.DynamicLibrary.open(nativePath);
  final execute = library.lookupFunction<
      _ExecuteRequestNative, _ExecuteRequestNative>(
    raw
        ? 'openkache_client_execute_raw_with_request_id'
        : 'openkache_client_execute_with_request_id',
  );
  final executeMutation = library.lookupFunction<
      _ExecuteMutationRequestNative, _ExecuteMutationRequestNative>(
    raw
        ? 'openkache_client_execute_raw_with_request_id_and_mutation_id'
        : 'openkache_client_execute_with_request_id_and_mutation_id',
  );
  final resultData = library.lookupFunction<_ResultDataNative, _ResultDataNative>(
    'openkache_client_result_data',
  );
  final resultLength = library.lookupFunction<_ResultLengthNative, _ResultLengthNative>(
    'openkache_client_result_data_length',
  );
  final resultKind = library.lookupFunction<_ResultKindNative, _ResultKindNative>(
    'openkache_client_result_kind',
  );
  final errorMetadata = library.lookupFunction<
      _ResultMetadataNative, _ResultMetadataNative>(
    'openkache_client_result_error_metadata',
  );
  final freeResult = library.lookupFunction<_FreeNative, _FreeNative>(
    'openkache_client_result_free',
  );
  final buffers = <_NativeBytes>[];
  try {
    final keyBuffer = _nativeBytes(key, buffers);
    final valueBuffer = _nativeBytes(value, buffers);
    final mutationBuffer = _nativeBytes(mutationId, buffers);
    final client = ffi.Pointer<ffi.Void>.fromAddress(clientAddress);
    final result = mutationId.isEmpty
        ? execute(
            client,
            requestId,
            operation,
            keyBuffer.pointer,
            keyBuffer.length,
            valueBuffer.pointer,
            valueBuffer.length,
            condition,
            ttlEnabled,
            ttlMilliseconds,
          )
        : executeMutation(
            client,
            requestId,
            operation,
            keyBuffer.pointer,
            keyBuffer.length,
            valueBuffer.pointer,
            valueBuffer.length,
            condition,
            ttlEnabled,
            ttlMilliseconds,
            mutationBuffer.pointer,
            mutationBuffer.length,
          );
    if (result == ffi.nullptr) {
      throw OpenKacheError('native operation returned null');
    }
    try {
      final kind = resultKind(result);
      final length = resultLength(result);
      final payload = length == 0
          ? Uint8List(0)
          : Uint8List.fromList(resultData(result).asTypedList(length));
      List<Object?>? metadata;
      if (kind == _resultError) {
        final metadataPointer = calloc<_ErrorMetadata>();
        try {
          if (errorMetadata(result, metadataPointer) != 0) {
            final value = metadataPointer.ref;
            metadata = <Object?>[
              value.code,
              value.operation,
              value.phase,
              value.backend,
              value.retryable != 0,
              value.ambiguous != 0,
              _readMutationId(value.mutationId, value.mutationIdLength),
            ];
          }
        } finally {
          calloc.free(metadataPointer);
        }
      }
      return <Object?>[kind, payload, metadata];
    } finally {
      freeResult(result);
    }
  } finally {
    for (final buffer in buffers) {
      _zeroizeNativeBytes(buffer);
      calloc.free(buffer.pointer);
    }
  }
}

ErrorMetadata _metadataFromList(List<Object?> value) => ErrorMetadata(
      code: value[0]! as int,
      operation: value[1]! as int,
      phase: value[2]! as int,
      backend: value[3]! as int,
      retryable: value[4]! as bool,
      ambiguous: value[5]! as bool,
      mutationId: value.length > 6 && value[6] is List
          ? Uint8List.fromList(value[6]! as List<int>)
          : null,
    );

Uint8List? _readMutationId(ffi.Array<ffi.Uint8> value, int length) {
  if (length == 0) return null;
  final bounded = length > _mutationIdBytes ? _mutationIdBytes : length;
  return Uint8List.fromList(
    List<int>.generate(bounded, (index) => value[index]),
  );
}
