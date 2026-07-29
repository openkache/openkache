/**
 * Length-prefixed IPC shared by the Node.js client and Rust transport helper.
 */
export declare const OPERATION_PING = 1;
export declare const OPERATION_GET = 2;
export declare const OPERATION_SET = 3;
export declare const OPERATION_DELETE = 4;
export declare const OPERATION_STATS = 5;
export declare const OPERATION_SYNC = 6;
export declare const RESULT_OK = 1;
export declare const RESULT_VALUE = 2;
export declare const RESULT_NOT_FOUND = 3;
export declare const RESULT_CREATED = 4;
export declare const RESULT_REPLACED = 5;
export declare const RESULT_DELETED = 6;
export declare const RESULT_NOT_DELETED = 7;
export declare const RESULT_CONNECTED = 8;
export declare const RESULT_NOT_STORED = 9;
export interface Helper_Identity {
    readonly certificate_chain: readonly Uint8Array[];
    readonly private_key: Uint8Array;
}
export interface Helper_Connection_Options {
    readonly address: string;
    readonly server_name: string;
    readonly certificate: Uint8Array;
    readonly identity?: Helper_Identity;
    readonly encryption_key: Uint8Array;
    readonly compression_enabled: boolean;
    readonly compression_level: number;
    readonly minimum_input_size: number;
    readonly minimum_savings: number;
    readonly connect_timeout_ms: number;
    readonly request_timeout_ms: number;
}
export interface Helper_Execute_Request {
    readonly operation: number;
    readonly condition: 0 | 1 | 2;
    readonly ttl_ms: number;
    readonly key: Uint8Array;
    readonly value: Uint8Array;
}
export interface Helper_Response {
    readonly request_id: number;
    readonly ok: boolean;
    readonly result_kind: number;
    readonly payload: Uint8Array;
}
export declare function encode_connect_request(request_id: number, options: Helper_Connection_Options): Uint8Array;
export declare function encode_execute_request(request_id: number, request: Helper_Execute_Request): Uint8Array;
export declare function encode_close_request(request_id: number): Uint8Array;
export declare class Helper_Response_Decoder {
    #private;
    push(chunk: Uint8Array): readonly Helper_Response[];
    finish(): void;
}
export declare function decode_helper_error(payload: Uint8Array): string;
//# sourceMappingURL=helper-protocol.d.ts.map