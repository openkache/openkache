// Generated from the OpenKache Smithy contract. Do not edit.

import Foundation

/// Values defined by the Smithy SetCondition shape.
public enum Smithy_Set_Condition: String, Equatable, Sendable {
  case ifAbsent = "if_absent"
  case ifPresent = "if_present"
}

/// Values defined by the Smithy SetOutcome shape.
public enum Smithy_Set_Outcome: String, Equatable, Sendable {
  case created = "created"
  case replaced = "replaced"
  case notStored = "not_stored"
}

/// Smithy DeleteInput structure.
public struct Smithy_Delete_Input: Equatable, Sendable {
  /// Smithy itemId member.
  public let itemId: Data

  public init(
    itemId: Data
  ) {
    self.itemId = itemId
  }
}

/// Smithy DeleteOutput structure.
public struct Smithy_Delete_Output: Equatable, Sendable {
  /// Smithy deleted member.
  public let deleted: Bool

  public init(
    deleted: Bool
  ) {
    self.deleted = deleted
  }
}

/// Smithy GetInput structure.
public struct Smithy_Get_Input: Equatable, Sendable {
  /// Smithy itemId member.
  public let itemId: Data

  public init(
    itemId: Data
  ) {
    self.itemId = itemId
  }
}

/// Smithy GetOutput structure.
public struct Smithy_Get_Output: Equatable, Sendable {
  /// Smithy value member.
  public let value: Data?

  public init(
    value: Data? = nil
  ) {
    self.value = value
  }
}

/// Smithy PingInput structure.
public struct Smithy_Ping_Input: Equatable, Sendable {
  public init() {}
}

/// Smithy PingOutput structure.
public struct Smithy_Ping_Output: Equatable, Sendable {
  public init() {}
}

/// Smithy SetInput structure.
public struct Smithy_Set_Input: Equatable, Sendable {
  /// Smithy itemId member.
  public let itemId: Data
  /// Smithy value member.
  public let value: Data
  /// Smithy condition member.
  public let condition: Smithy_Set_Condition?
  /// Smithy ttlMilliseconds member.
  public let ttlMilliseconds: Int64?

  public init(
    itemId: Data,
    value: Data,
    condition: Smithy_Set_Condition? = nil,
    ttlMilliseconds: Int64? = nil
  ) {
    self.itemId = itemId
    self.value = value
    self.condition = condition
    self.ttlMilliseconds = ttlMilliseconds
  }
}

/// Smithy SetOutput structure.
public struct Smithy_Set_Output: Equatable, Sendable {
  /// Smithy outcome member.
  public let outcome: Smithy_Set_Outcome

  public init(
    outcome: Smithy_Set_Outcome
  ) {
    self.outcome = outcome
  }
}

/// Smithy StatsInput structure.
public struct Smithy_Stats_Input: Equatable, Sendable {
  public init() {}
}

/// Smithy StatsOutput structure.
public struct Smithy_Stats_Output: Equatable, Sendable {
  /// Smithy json member.
  public let json: String

  public init(
    json: String
  ) {
    self.json = json
  }
}

/// Smithy SyncInput structure.
public struct Smithy_Sync_Input: Equatable, Sendable {
  public init() {}
}

/// Smithy SyncOutput structure.
public struct Smithy_Sync_Output: Equatable, Sendable {
  public init() {}
}

/// Operations defined by the OpenKache Smithy service.
public protocol Smithy_OpenKache_Api: Sendable {
  /// Invokes the Smithy Ping operation.
  func ping(
    _ input: Smithy_Ping_Input
  ) async throws -> Smithy_Ping_Output
  /// Invokes the Smithy Get operation.
  func get(
    _ input: Smithy_Get_Input
  ) async throws -> Smithy_Get_Output
  /// Invokes the Smithy Set operation.
  func set(
    _ input: Smithy_Set_Input
  ) async throws -> Smithy_Set_Output
  /// Invokes the Smithy Delete operation.
  func delete(
    _ input: Smithy_Delete_Input
  ) async throws -> Smithy_Delete_Output
  /// Invokes the Smithy Stats operation.
  func stats(
    _ input: Smithy_Stats_Input
  ) async throws -> Smithy_Stats_Output
  /// Invokes the Smithy Sync operation.
  func sync(
    _ input: Smithy_Sync_Input
  ) async throws -> Smithy_Sync_Output
}

/// Operation identifiers assigned by the Smithy wire contract.
public enum Smithy_Opcode: UInt8, Equatable, Sendable {
  case ping = 1
  case get = 2
  case set = 3
  case delete = 4
  case stats = 5
  case sync = 6
}

/// Wire and value-format identifiers shared by all language bindings.
public enum Smithy_Value_Format: Sendable {
  public static let protocolAlpn: String = "openkache/1"
  public static let itemIdBytes: Int = 32
  public static let maxValueBytes: Int = 67108864
  public static let version: Int = 1
  public static let versionBytes: [UInt8] = [1]
  public static let maxVu128Bytes: Int = 17
  public static let formatByteBytes: Int = 1
  public static let maxVaruintBytes: Int = 9
  public static let setTtlFlag: UInt8 = 1
  public static let setIfAbsentFlag: UInt8 = 2
  public static let setIfPresentFlag: UInt8 = 4
  public static let formatCompressionMask: UInt8 = 15
  public static let formatEncryptionShift: UInt8 = 4
  public static let serializationRaw: UInt8 = 0
  public static let serializationJson: UInt8 = 1
  public static let compressionNone: UInt8 = 0
  public static let compressionZstandard: UInt8 = 1
  public static let encryptionNone: UInt8 = 0
  public static let encryptionCompact: UInt8 = 1
  public static let encryptionRobust: UInt8 = 2
  public static let compactSyntheticIvBytes: Int = 16
  public static let robustNonceBytes: Int = 12
  public static let robustTagBytes: Int = 16
  public static let dataProtectionKeyBytes: Int = 32
  public static let itemIdRootContext: String = "OpenKache client item key root v1"
  public static let aadDomain: String = "openkache/value-format/aad/v1"
  public static let valueRootContext: String = "OpenKache value format v1 root key"
  public static let compactMacContext: String = "OpenKache value format v1 AES-256-SIV-CMAC MAC key"
  public static let compactEncryptionContext: String = "OpenKache value format v1 AES-256-SIV-CMAC encryption key"
  public static let robustContext: String = "OpenKache value format v1 AES-256-GCM-SIV key"
}

/// Native ABI discriminators shared by every language adapter.
public enum Smithy_Connection_State: UInt32, Sendable {
  case connected = 0
  case reconnecting = 1
  case disconnected = 2
  case closed = 3
  case unknown = 4
}

/// Native ABI discriminators shared by every language adapter.
public enum Smithy_Native_Contract: Sendable {
  public static let abiVersion: UInt32 = 1
  public static let operationReconnect: UInt32 = 4294967041
  public static let operationConnectionState: UInt32 = 4294967042
  public static let resultError: UInt32 = 0
  public static let resultOk: UInt32 = 1
  public static let resultValue: UInt32 = 2
  public static let resultNotFound: UInt32 = 3
  public static let resultCreated: UInt32 = 4
  public static let resultReplaced: UInt32 = 5
  public static let resultDeleted: UInt32 = 6
  public static let resultNotDeleted: UInt32 = 7
  public static let resultConnected: UInt32 = 8
  public static let resultNotStored: UInt32 = 9
  public static let resultConnectionState: UInt32 = 10
  public static let setConditionNone: UInt32 = 0
  public static let setConditionIfAbsent: UInt32 = 1
  public static let setConditionIfPresent: UInt32 = 2
  public static let connectionStateConnected: UInt32 = 0
  public static let connectionStateReconnecting: UInt32 = 1
  public static let connectionStateDisconnected: UInt32 = 2
  public static let connectionStateClosed: UInt32 = 3
  public static let connectionStateUnknown: UInt32 = 4
}
