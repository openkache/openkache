library;

import 'dart:convert';
import 'dart:ffi' as ffi;
import 'dart:io';

import 'package:ffi/ffi.dart';

import 'generated_local/smithy_api.dart';
import 'generated_local/smithy_contract.dart';

/// Failure reported by the shared Rust client-core ABI.
final class EchoClientException implements Exception {
  const EchoClientException(this.message, [this.cause]);

  final String message;
  final Object? cause;

  @override
  String toString() => cause == null ? message : '$message: $cause';
}

/// Dart client for the experimental Smithy `Echo` operation.
///
/// QUIC, TLS, framing, retries, and native result ownership remain in the
/// shared Rust client core. This adapter only marshals Dart strings through
/// the stable C ABI.
abstract interface class OpenKacheClient implements SmithyEchoApi {}

final class EchoClient implements OpenKacheClient {
  EchoClient._(this._api, this._handle);

  final _NativeApi _api;
  ffi.Pointer<_Client> _handle;
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
    final api = _NativeApi.open(libraryPath);
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
      final result = api.connect(
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
      );
      final nativeResult = api.readResult(result, takeClient: true);
      if (nativeResult.kind != smithyResultConnected ||
          nativeResult.client == null ||
          nativeResult.client == ffi.nullptr) {
        throw const EchoClientException(
          'native client did not return a connected handle',
        );
      }
      return EchoClient._(api, nativeResult.client!);
    } finally {
      addressBuffer.close();
      serverBuffer.close();
      certificateBuffer.close();
      keyBuffer.close();
    }
  }

  @override
  Future<EchoOutput> echo(EchoInput input) async {
    _ensureOpen();
    final payload = _echoBytes(utf8.encode(input.message));
    try {
      return EchoOutput(message: utf8.decode(payload, allowMalformed: false));
    } on FormatException catch (error) {
      throw EchoClientException('ECHO response is not valid UTF-8', error);
    }
  }

  /// Sends one message and returns the echoed text.
  Future<String> echoMessage(String message) async =>
      (await echo(EchoInput(message: message))).message;

  /// Releases the shared native client handle.
  void close() {
    if (_closed) {
      return;
    }
    _closed = true;
    _api.clientFree(_handle);
    _handle = ffi.nullptr;
  }

  List<int> _echoBytes(List<int> message) {
    _ensureOpen();
    final applicationKey = _Buffer(const <int>[]);
    final value = _Buffer(message);
    try {
      final result = _api.execute(
        _handle,
        smithyOperationEcho,
        applicationKey.pointer,
        applicationKey.length,
        value.pointer,
        value.length,
        smithySetConditionAny,
        0,
        0,
      );
      final nativeResult = _api.readResult(result);
      if (nativeResult.kind != smithyResultValue) {
        throw const EchoClientException(
          'native client returned an invalid ECHO result',
        );
      }
      return nativeResult.payload;
    } finally {
      applicationKey.close();
      value.close();
    }
  }

  void _ensureOpen() {
    if (_closed || _handle == ffi.nullptr) {
      throw const EchoClientException('OpenKache client is closed');
    }
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

final class _Client extends ffi.Opaque {}

final class _Result extends ffi.Opaque {}

typedef _ConnectNative =
    ffi.Pointer<_Result> Function(
      ffi.Pointer<ffi.Uint8>,
      ffi.UintPtr,
      ffi.Pointer<ffi.Uint8>,
      ffi.UintPtr,
      ffi.Pointer<ffi.Uint8>,
      ffi.UintPtr,
      ffi.Pointer<ffi.Uint8>,
      ffi.UintPtr,
      ffi.Uint8,
      ffi.Int32,
      ffi.UintPtr,
      ffi.UintPtr,
      ffi.Uint64,
      ffi.Uint64,
    );

typedef _ConnectDart =
    ffi.Pointer<_Result> Function(
      ffi.Pointer<ffi.Uint8>,
      int,
      ffi.Pointer<ffi.Uint8>,
      int,
      ffi.Pointer<ffi.Uint8>,
      int,
      ffi.Pointer<ffi.Uint8>,
      int,
      int,
      int,
      int,
      int,
      int,
      int,
    );

typedef _ExecuteNative =
    ffi.Pointer<_Result> Function(
      ffi.Pointer<_Client>,
      ffi.Uint32,
      ffi.Pointer<ffi.Uint8>,
      ffi.UintPtr,
      ffi.Pointer<ffi.Uint8>,
      ffi.UintPtr,
      ffi.Uint32,
      ffi.Uint8,
      ffi.Uint64,
    );

typedef _ExecuteDart =
    ffi.Pointer<_Result> Function(
      ffi.Pointer<_Client>,
      int,
      ffi.Pointer<ffi.Uint8>,
      int,
      ffi.Pointer<ffi.Uint8>,
      int,
      int,
      int,
      int,
    );

final class _NativeApi {
  _NativeApi(ffi.DynamicLibrary library)
    : abiVersion = library.lookupFunction<_AbiVersionNative, _AbiVersionDart>(
        'openkache_client_abi_version',
      ),
      connect = library.lookupFunction<_ConnectNative, _ConnectDart>(
        'openkache_client_connect',
      ),
      execute = library.lookupFunction<_ExecuteNative, _ExecuteDart>(
        'openkache_client_execute',
      ),
      resultKind = library.lookupFunction<_ResultKindNative, _ResultKindDart>(
        'openkache_client_result_kind',
      ),
      resultData = library.lookupFunction<_ResultDataNative, _ResultDataDart>(
        'openkache_client_result_data',
      ),
      resultLength = library
          .lookupFunction<_ResultLengthNative, _ResultLengthDart>(
            'openkache_client_result_data_length',
          ),
      resultTakeClient = library
          .lookupFunction<_ResultTakeClientNative, _ResultTakeClientDart>(
            'openkache_client_result_take_client',
          ),
      resultFree = library.lookupFunction<_ResultFreeNative, _ResultFreeDart>(
        'openkache_client_result_free',
      ),
      clientFree = library.lookupFunction<_ClientFreeNative, _ClientFreeDart>(
        'openkache_client_free',
      );

  final int Function() abiVersion;
  final _ConnectDart connect;
  final _ExecuteDart execute;
  final int Function(ffi.Pointer<_Result>) resultKind;
  final ffi.Pointer<ffi.Uint8> Function(ffi.Pointer<_Result>) resultData;
  final int Function(ffi.Pointer<_Result>) resultLength;
  final ffi.Pointer<_Client> Function(ffi.Pointer<_Result>) resultTakeClient;
  final void Function(ffi.Pointer<_Result>) resultFree;
  final void Function(ffi.Pointer<_Client>) clientFree;

  static _NativeApi open(String? configuredPath) {
    final path =
        configuredPath ??
        Platform.environment['OPENKACHE_CLIENT_NATIVE'] ??
        switch (Platform.operatingSystem) {
          'linux' => 'libopenkache_client_core.so',
          'macos' => 'libopenkache_client_core.dylib',
          'windows' => 'openkache_client_core.dll',
          _ => throw UnsupportedError(
            'unsupported platform ${Platform.operatingSystem}',
          ),
        };
    try {
      return _NativeApi(ffi.DynamicLibrary.open(path));
    } on Object catch (error) {
      throw EchoClientException(
        'failed to load OpenKache native client',
        error,
      );
    }
  }

  _NativeResult readResult(
    ffi.Pointer<_Result> result, {
    bool takeClient = false,
  }) {
    if (result == ffi.nullptr) {
      throw const EchoClientException('native client returned a null result');
    }
    try {
      final kind = resultKind(result);
      final length = resultLength(result);
      if (length < 0 || length > 0x7fffffff) {
        throw const EchoClientException(
          'native client returned an oversized payload',
        );
      }
      final data = resultData(result);
      if (length != 0 && data == ffi.nullptr) {
        throw const EchoClientException(
          'native client returned a null payload pointer',
        );
      }
      final payload = length == 0
          ? <int>[]
          : data.asTypedList(length).toList(growable: false);
      final client = takeClient ? resultTakeClient(result) : ffi.nullptr;
      if (kind == smithyResultError) {
        throw EchoClientException(
          utf8.decode(payload, allowMalformed: true).isEmpty
              ? 'native client operation failed'
              : utf8.decode(payload, allowMalformed: true),
        );
      }
      return _NativeResult(kind, payload, client);
    } finally {
      resultFree(result);
    }
  }
}

final class _NativeResult {
  const _NativeResult(this.kind, this.payload, this.client);

  final int kind;
  final List<int> payload;
  final ffi.Pointer<_Client>? client;
}

typedef _AbiVersionNative = ffi.Uint32 Function();
typedef _AbiVersionDart = int Function();
typedef _ResultKindNative = ffi.Uint32 Function(ffi.Pointer<_Result>);
typedef _ResultKindDart = int Function(ffi.Pointer<_Result>);
typedef _ResultDataNative =
    ffi.Pointer<ffi.Uint8> Function(ffi.Pointer<_Result>);
typedef _ResultDataDart = ffi.Pointer<ffi.Uint8> Function(ffi.Pointer<_Result>);
typedef _ResultLengthNative = ffi.UintPtr Function(ffi.Pointer<_Result>);
typedef _ResultLengthDart = int Function(ffi.Pointer<_Result>);
typedef _ResultTakeClientNative =
    ffi.Pointer<_Client> Function(ffi.Pointer<_Result>);
typedef _ResultTakeClientDart =
    ffi.Pointer<_Client> Function(ffi.Pointer<_Result>);
typedef _ResultFreeNative = ffi.Void Function(ffi.Pointer<_Result>);
typedef _ResultFreeDart = void Function(ffi.Pointer<_Result>);
typedef _ClientFreeNative = ffi.Void Function(ffi.Pointer<_Client>);
typedef _ClientFreeDart = void Function(ffi.Pointer<_Client>);
