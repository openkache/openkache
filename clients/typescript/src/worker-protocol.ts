export const RESULT_ERROR = 0
export const RESULT_OK = 1
export const RESULT_VALUE = 2
export const RESULT_NOT_FOUND = 3
export const RESULT_CREATED = 4
export const RESULT_REPLACED = 5
export const RESULT_DELETED = 6
export const RESULT_NOT_DELETED = 7
export const RESULT_CONNECTED = 8
export const RESULT_NOT_STORED = 9

export const OPERATION_PING = 1
export const OPERATION_GET = 2
export const OPERATION_SET = 3
export const OPERATION_DELETE = 4
export const OPERATION_STATS = 5
export const OPERATION_SYNC = 6

export interface Worker_Connection_Options {
  readonly address: string
  readonly certificate: Uint8Array
  readonly encryption_key: Uint8Array
  readonly server_name: string
  readonly compression_enabled: boolean
  readonly compression_level: number
  readonly minimum_input_size: number
  readonly minimum_savings: number
  readonly library_path: string
}

export type Worker_Request_Body =
  | {
      readonly kind: "connect"
      readonly options: Worker_Connection_Options
    }
  | {
      readonly kind: "execute"
      readonly operation: number
      readonly key: Uint8Array
      readonly value: Uint8Array
      readonly set_condition: number
      readonly ttl_ms: number
    }
  | {
      readonly kind: "close"
    }

export type Worker_Request = Worker_Request_Body & {
  readonly request_id: number
}

export interface Worker_Success_Response {
  readonly request_id: number
  readonly ok: true
  readonly result_kind: number
  readonly payload: Uint8Array
}

export interface Worker_Error_Response {
  readonly request_id: number
  readonly ok: false
  readonly message: string
}

export type Worker_Response = Worker_Success_Response | Worker_Error_Response
