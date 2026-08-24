#ifndef OPENKACHE_CLIENT_HPP
#define OPENKACHE_CLIENT_HPP

#include <array>
#include <cstdint>
#include <optional>
#include <span>
#include <stdexcept>
#include <string>
#include <string_view>
#include <thread>
#include <tuple>
#include <utility>
#include <vector>

#include <openkache/client.h>

#if defined(_WIN32)
#include <windows.h>
#endif

namespace openkache {

using Byte = std::uint8_t;
using Bytes = std::vector<Byte>;

/// Native C++ exception carrying a core or argument failure.
class Error : public std::runtime_error {
public:
    explicit Error(const std::string& message)
        : std::runtime_error(message) {}
};

// The transport selector was added as an optional ABI symbol. Resolve it at
// runtime so the compatibility-default path still links against older native
// libraries that predate TLS-over-TCP.
#if !defined(_WIN32) && (defined(__GNUC__) || defined(__clang__))
extern "C" openkache_client_result_t* openkache_client_connect_transport(
    const openkache_client_connect_options_t*,
    std::uint32_t) __attribute__((weak));
#endif

using Connect_Transport_Function = openkache_client_result_t* (*)(
    const openkache_client_connect_options_t*,
    std::uint32_t);

inline Connect_Transport_Function optional_connect_transport() noexcept {
#if defined(_WIN32)
    HMODULE module = nullptr;
    if (!GetModuleHandleExA(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
            reinterpret_cast<LPCSTR>(&openkache_client_abi_version),
            &module)) {
        return nullptr;
    }
    return reinterpret_cast<Connect_Transport_Function>(
        GetProcAddress(module, "openkache_client_connect_transport"));
#elif defined(__GNUC__) || defined(__clang__)
    return openkache_client_connect_transport;
#else
    return nullptr;
#endif
}
/// A mutation crossed the native cancellation/admission boundary.
class Unknown_Mutation_Error : public Error {
public:
    explicit Unknown_Mutation_Error(const std::string& message)
        : Error(message) {}
};

/// A read-only request was canceled before native admission.
class Canceled_Error : public Error {
public:
    explicit Canceled_Error(const std::string& message)
        : Error(message) {}
};

/// Atomic existence condition for one SET operation.
enum class Set_Condition : std::uint32_t {
    Any = OPENKACHE_SMITHY_FFI_SET_CONDITION_ANY,
    If_Absent = OPENKACHE_SMITHY_FFI_SET_CONDITION_IF_ABSENT,
    If_Present = OPENKACHE_SMITHY_FFI_SET_CONDITION_IF_PRESENT,
};

/// Item-level expiration selection for one SET operation.
enum class Expiration_Mode : Byte {
    Inherit = OPENKACHE_SMITHY_SET_INHERIT_EXPIRATION_BITS,
    No_Expiry = OPENKACHE_SMITHY_SET_NO_EXPIRY_BITS,
    Explicit_Ttl = OPENKACHE_SMITHY_SET_EXPLICIT_TTL_BITS,
};

/// Item-level capacity-eviction selection for one SET operation.
enum class Eviction_Mode : Byte {
    Inherit = OPENKACHE_SMITHY_SET_INHERIT_EVICTION_BITS,
    Evictable = OPENKACHE_SMITHY_SET_EVICTABLE_BITS,
    Eviction_Protected = OPENKACHE_SMITHY_SET_EVICTION_PROTECTED_BITS,
};

/// Namespace-level expiration default.
enum class Namespace_Expiration_Default : std::uint32_t {
    No_Expiry = OPENKACHE_SMITHY_FFI_NAMESPACE_DEFAULT_EXPIRATION_NO_EXPIRY,
    Fixed_Ttl = OPENKACHE_SMITHY_FFI_NAMESPACE_DEFAULT_EXPIRATION_FIXED_TTL,
};

/// Namespace-level capacity-eviction default.
enum class Namespace_Eviction_Default : std::uint32_t {
    Evictable = OPENKACHE_SMITHY_FFI_NAMESPACE_DEFAULT_EVICTION_EVICTABLE,
    Eviction_Protected = OPENKACHE_SMITHY_FFI_NAMESPACE_DEFAULT_EVICTION_PROTECTED,
};

/// Whether namespace policy defaults may be overridden by individual SETs.
enum class Namespace_Override_Policy : std::uint32_t {
    Disallowed = OPENKACHE_SMITHY_FFI_NAMESPACE_OVERRIDE_DISALLOWED,
    Allowed = OPENKACHE_SMITHY_FFI_NAMESPACE_OVERRIDE_ALLOWED,
};

/// Namespace policy supplied when creating or replacing a namespace policy.
struct Namespace_Policy {
    Namespace_Expiration_Default default_expiration =
        Namespace_Expiration_Default::No_Expiry;
    std::optional<std::uint64_t> default_ttl_ms;
    Namespace_Override_Policy expiration_override =
        Namespace_Override_Policy::Allowed;
    Namespace_Eviction_Default default_eviction =
        Namespace_Eviction_Default::Evictable;
    Namespace_Override_Policy eviction_override =
        Namespace_Override_Policy::Allowed;
};

/// Server-assigned namespace identity and its current policy.
struct Namespace_Descriptor {
    std::uint64_t namespace_id;
    std::uint64_t revision;
    Namespace_Policy policy;
};

/// Result of opening a namespace.
struct Namespace_Open_Result {
    Namespace_Descriptor descriptor;
    bool created;
};

/// Authenticated-encryption profile for formatted values.
enum class Encryption : std::uint32_t {
    Compact = OPENKACHE_SMITHY_VALUE_ENCRYPTION_COMPACT,
    Robust = OPENKACHE_SMITHY_VALUE_ENCRYPTION_ROBUST,
};

/// Successful SET outcome.
enum class Set_Outcome {
    Created,
    Replaced,
    Not_Stored,
};

/// Best-effort state of the native connection worker.
enum class Connection_State : std::uint32_t {
    Connected = OPENKACHE_SMITHY_FFI_CONNECTION_STATE_CONNECTED,
    Reconnecting = OPENKACHE_SMITHY_FFI_CONNECTION_STATE_RECONNECTING,
    Disconnected = OPENKACHE_SMITHY_FFI_CONNECTION_STATE_DISCONNECTED,
    Closed = OPENKACHE_SMITHY_FFI_CONNECTION_STATE_CLOSED,
    Unknown = OPENKACHE_SMITHY_FFI_CONNECTION_STATE_UNKNOWN,
};

/// Native transport and server-trust selector.
enum class Transport : std::uint32_t {
    Quic = OPENKACHE_CLIENT_TRANSPORT_QUIC,
    Tls_Tcp = OPENKACHE_CLIENT_TRANSPORT_TLS_TCP,
    Quic_Insecure = OPENKACHE_CLIENT_TRANSPORT_QUIC_INSECURE,
    Tls_Tcp_Insecure = OPENKACHE_CLIENT_TRANSPORT_TLS_TCP_INSECURE,
};

/// Optional behavior for one SET operation.
struct Set_Options {
    Set_Condition condition = Set_Condition::Any;
    std::optional<Expiration_Mode> expiration_mode;
    std::optional<Eviction_Mode> eviction_mode;
    std::optional<std::uint64_t> ttl_ms;
};

/// Values passed to `Client::connect`.
struct Connect_Options {
    /// Hostname or numeric address with a transport port.
    std::string address;
    /// TLS certificate identity. Empty selects the host in `address`.
    std::string server_name;
    /// One DER certificate or PEM chain. Empty selects system trust roots.
    std::vector<Byte> certificate;
    /// Optional exact 32-byte root key. Empty selects unprotected values.
    std::vector<Byte> data_protection_key;
    /// Automatic level-1 Zstandard compression is enabled by default. Set to
    /// false to preserve the exact uncompressed formatted-value behavior.
    bool compression_enabled = true;
    std::int32_t compression_level = OPENKACHE_SMITHY_DEFAULT_ZSTANDARD_LEVEL;
    std::size_t minimum_input_size = OPENKACHE_SMITHY_DEFAULT_ZSTANDARD_MINIMUM_INPUT_BYTES;
    std::size_t minimum_savings = OPENKACHE_SMITHY_DEFAULT_ZSTANDARD_MINIMUM_SAVINGS_BYTES;
    std::vector<Byte> client_certificate_chain;
    std::vector<Byte> client_private_key;
    Encryption encryption = Encryption::Robust;
    std::uint64_t connect_timeout_ms =
        OPENKACHE_SMITHY_DEFAULT_CONNECT_TIMEOUT_MILLISECONDS;
    std::uint64_t request_timeout_ms =
        OPENKACHE_SMITHY_DEFAULT_REQUEST_TIMEOUT_MILLISECONDS;
    std::size_t retry_max_attempts = OPENKACHE_SMITHY_DEFAULT_RETRY_MAX_ATTEMPTS;
    std::size_t max_in_flight = OPENKACHE_SMITHY_DEFAULT_MAX_IN_FLIGHT;
    Transport transport = Transport::Quic;
};

/// RAII C++ client over the shared core C ABI.
///
/// A client owns one native worker and is movable but not copyable. Every method is synchronous
/// from the caller's perspective; network work is performed by the dedicated Rust worker.
class Client {
public:
    Client() noexcept = default;

    /// Takes ownership of a native client pointer.
    explicit Client(openkache_client_t* client) noexcept
        : client_(client) {}

    Client(const Client&) = delete;
    Client& operator=(const Client&) = delete;

    Client(Client&& other) noexcept
        : client_(std::exchange(other.client_, nullptr)) {}

    Client& operator=(Client&& other) noexcept {
        if (this != &other) {
            close();
            client_ = std::exchange(other.client_, nullptr);
        }
        return *this;
    }

    ~Client() noexcept {
        close();
    }

    /// Connects with the supplied certificate, protection key, and deadlines.
    static Client connect(const Connect_Options& options) {
        const auto native_abi_version = openkache_client_abi_version();
        if (native_abi_version != OPENKACHE_CLIENT_ABI_VERSION) {
            throw Error(
                "unsupported OpenKache client ABI version "
                + std::to_string(native_abi_version));
        }
        const auto* certificate = options.certificate.empty()
            ? nullptr
            : options.certificate.data();
        const auto* key = options.data_protection_key.data();
        const auto* client_certificate_chain =
            options.client_certificate_chain.empty()
                ? nullptr
                : options.client_certificate_chain.data();
        const auto* client_private_key = options.client_private_key.empty()
            ? nullptr
            : options.client_private_key.data();
        if (options.transport != Transport::Quic) {
            const auto connect_transport = optional_connect_transport();
            if (connect_transport == nullptr) {
                throw Error(
                    "native OpenKache client does not export the optional transport selector");
            }
            const openkache_client_connect_options_t native_options{
                reinterpret_cast<const Byte*>(options.address.data()),
                options.address.size(),
                reinterpret_cast<const Byte*>(options.server_name.data()),
                options.server_name.size(),
                certificate,
                options.certificate.size(),
                client_certificate_chain,
                options.client_certificate_chain.size(),
                client_private_key,
                options.client_private_key.size(),
                key,
                options.data_protection_key.size(),
                static_cast<std::uint8_t>(options.compression_enabled ? 1u : 0u),
                options.compression_level,
                options.minimum_input_size,
                options.minimum_savings,
                static_cast<std::uint32_t>(options.encryption),
                options.connect_timeout_ms,
                options.request_timeout_ms,
                options.retry_max_attempts,
                options.max_in_flight,
            };
            return connect_result(connect_transport(
                &native_options, static_cast<std::uint32_t>(options.transport)));
        }
        openkache_client_result_t* result =
            openkache_client_connect_ex(
                reinterpret_cast<const Byte*>(options.address.data()),
                options.address.size(),
                reinterpret_cast<const Byte*>(options.server_name.data()),
                options.server_name.size(),
                certificate,
                options.certificate.size(),
                client_certificate_chain,
                options.client_certificate_chain.size(),
                client_private_key,
                options.client_private_key.size(),
                key,
                options.data_protection_key.size(),
                static_cast<std::uint8_t>(options.compression_enabled ? 1u : 0u),
                options.compression_level,
                options.minimum_input_size,
                options.minimum_savings,
                static_cast<std::uint32_t>(options.encryption),
                options.retry_max_attempts,
                options.max_in_flight,
                options.connect_timeout_ms,
                options.request_timeout_ms);
        return connect_result(result);
    }

    /// Returns whether this object owns an open native handle.
    explicit operator bool() const noexcept {
        return client_ != nullptr;
    }

    /// Closes the native worker. Repeated calls are safe.
    void close() noexcept {
        if (client_ != nullptr) {
            openkache_client_free(client_);
            client_ = nullptr;
        }
    }

    /// Returns the latest best-effort native connection state.
    Connection_State connection_state() const noexcept {
        if (client_ == nullptr) {
            return Connection_State::Closed;
        }
        const auto state = openkache_client_connection_state(client_);
        switch (state) {
        case OPENKACHE_SMITHY_FFI_CONNECTION_STATE_CONNECTED:
            return Connection_State::Connected;
        case OPENKACHE_SMITHY_FFI_CONNECTION_STATE_RECONNECTING:
            return Connection_State::Reconnecting;
        case OPENKACHE_SMITHY_FFI_CONNECTION_STATE_DISCONNECTED:
            return Connection_State::Disconnected;
        case OPENKACHE_SMITHY_FFI_CONNECTION_STATE_CLOSED:
            return Connection_State::Closed;
        default:
            return Connection_State::Unknown;
        }
    }

    /// Replaces a failed connection without replaying an operation.
    void reconnect() const {
        const auto result = execute(
            OPENKACHE_SMITHY_FFI_OPERATION_RECONNECT,
            OPENKACHE_SMITHY_FFI_KEY_SPEC_BYTES,
            {},
            {},
            Set_Options{});
        if (result.kind != OPENKACHE_SMITHY_FFI_RESULT_OK) {
            throw Error("OpenKache returned an invalid RECONNECT outcome");
        }
    }

    /// Verifies the connection.
    void ping() const {
        const auto result = execute(
            OPENKACHE_SMITHY_OPCODE_PING,
            OPENKACHE_SMITHY_FFI_KEY_SPEC_BYTES,
            {},
            {},
            Set_Options{});
        if (result.kind != OPENKACHE_SMITHY_FFI_RESULT_OK) {
            throw Error("OpenKache returned an invalid PING outcome");
        }
    }

    /// Retrieves a Bytes PortableKey value, or `std::nullopt` when absent.
    std::optional<Bytes> get(std::span<const Byte> key) const {
        return get_outcome(
            execute(
                OPENKACHE_SMITHY_OPCODE_GET,
                OPENKACHE_SMITHY_FFI_KEY_SPEC_BYTES,
                key,
                {},
                Set_Options{}),
            "GET");
    }

    /// Convenience overload for a Text PortableKey.
    std::optional<Bytes> get(std::string_view key) const {
        return get_outcome(
            execute(
                OPENKACHE_SMITHY_OPCODE_GET,
                OPENKACHE_SMITHY_FFI_KEY_SPEC_TEXT,
                as_bytes(key),
                {},
                Set_Options{}),
            "GET");
    }

    /// Stores a Bytes PortableKey value and returns the server outcome.
    Set_Outcome set(
        std::span<const Byte> key,
        std::span<const Byte> value,
        Set_Options options = {}) const {
        return set_outcome(
            execute(
                OPENKACHE_SMITHY_OPCODE_SET,
                OPENKACHE_SMITHY_FFI_KEY_SPEC_BYTES,
                key,
                value,
                options),
            "SET");
    }

    /// Convenience overload for a Text PortableKey and textual value bytes.
    Set_Outcome set(
        std::string_view key,
        std::string_view value,
        Set_Options options = {}) const {
        return set_outcome(
            execute(
                OPENKACHE_SMITHY_OPCODE_SET,
                OPENKACHE_SMITHY_FFI_KEY_SPEC_TEXT,
                as_bytes(key),
                as_bytes(value),
                options),
            "SET");
    }

    /// Deletes a Bytes PortableKey value and reports whether it existed.
    bool remove(std::span<const Byte> key) const {
        return delete_outcome(
            execute(
                OPENKACHE_SMITHY_OPCODE_DELETE,
                OPENKACHE_SMITHY_FFI_KEY_SPEC_BYTES,
                key,
                {},
                Set_Options{}));
    }

    /// Convenience overload for a Text PortableKey.
    bool remove(std::string_view key) const {
        return delete_outcome(
            execute(
                OPENKACHE_SMITHY_OPCODE_DELETE,
                OPENKACHE_SMITHY_FFI_KEY_SPEC_TEXT,
                as_bytes(key),
                {},
                Set_Options{}));
    }

    /// Retrieves exact bytes for a `0..=32`-byte protocol item ID.
    std::optional<Bytes> get_raw(std::span<const Byte> item_id) const {
        return get_outcome(
            execute(
                OPENKACHE_SMITHY_OPCODE_GET,
                OPENKACHE_SMITHY_FFI_KEY_SPEC_BYTES,
                item_id,
                {},
                Set_Options{},
                true),
            "raw GET");
    }

    /// Stores exact bytes for a `0..=32`-byte protocol item ID without value protection.
    Set_Outcome set_raw(
        std::span<const Byte> item_id,
        std::span<const Byte> value,
        Set_Options options = {}) const {
        return set_outcome(
            execute(
                OPENKACHE_SMITHY_OPCODE_SET,
                OPENKACHE_SMITHY_FFI_KEY_SPEC_BYTES,
                item_id,
                value,
                options,
                true),
            "raw SET");
    }

    /// Deletes a `0..=32`-byte protocol item ID without application-key derivation.
    bool remove_raw(std::span<const Byte> item_id) const {
        return delete_outcome(
            execute(
                OPENKACHE_SMITHY_OPCODE_DELETE,
                OPENKACHE_SMITHY_FFI_KEY_SPEC_BYTES,
                item_id,
                {},
                Set_Options{},
                true));
    }

    /// Returns the server's JSON statistics document.
    std::string experimental_stats() const {
        const auto result = execute(
            OPENKACHE_SMITHY_OPCODE_EXPERIMENTAL_STATS,
            OPENKACHE_SMITHY_FFI_KEY_SPEC_BYTES,
            {},
            {},
            Set_Options{});
        if (result.kind != OPENKACHE_SMITHY_FFI_RESULT_VALUE) {
            throw Error("OpenKache returned an invalid EXPERIMENTAL_STATS outcome");
        }
        if (result.payload.empty()) {
            return {};
        }
        return std::string(
            reinterpret_cast<const char*>(result.payload.data()),
            result.payload.size());
    }

    /// Waits for the server durability barrier.
    void experimental_sync() const {
        const auto result = execute(
            OPENKACHE_SMITHY_OPCODE_EXPERIMENTAL_SYNC,
            OPENKACHE_SMITHY_FFI_KEY_SPEC_BYTES,
            {},
            {},
            Set_Options{});
        if (result.kind != OPENKACHE_SMITHY_FFI_RESULT_OK) {
            throw Error("OpenKache returned an invalid EXPERIMENTAL_SYNC outcome");
        }
    }

    /// Resolves or creates a namespace by its UTF-8 name.
    Namespace_Open_Result namespace_open(
        std::string_view name,
        bool create_if_missing,
        std::optional<Namespace_Policy> policy = std::nullopt) const {
        if (client_ == nullptr) {
            throw Error("OpenKache client is closed");
        }
        if (name.size() > OPENKACHE_SMITHY_NAMESPACE_NAME_MAX_BYTES) {
            throw Error(
                "OpenKache namespace name exceeds "
                + std::to_string(OPENKACHE_SMITHY_NAMESPACE_NAME_MAX_BYTES)
                + " UTF-8 octets");
        }
        if (create_if_missing && !policy.has_value()) {
            throw Error("namespace policy is required when create_if_missing is true");
        }
        if (!create_if_missing && policy.has_value()) {
            throw Error("namespace policy requires create_if_missing");
        }
        const auto [flags, ttl_ms] = policy.has_value()
            ? namespace_policy_wire(*policy)
            : std::pair<Byte, std::uint64_t>{0, 0};
        const auto* name_data = name.empty()
            ? nullptr
            : reinterpret_cast<const Byte*>(name.data());
        auto* result = openkache_client_namespace_open(
            client_,
            name_data,
            name.size(),
            static_cast<Byte>(
                create_if_missing ? OPENKACHE_SMITHY_OPEN_CREATE_IF_MISSING : 0u),
            create_if_missing ? flags : 0,
            create_if_missing ? ttl_ms : 0);
        const auto operation = take_result(result);
        if (operation.kind != OPENKACHE_SMITHY_FFI_RESULT_OK
            && operation.kind != OPENKACHE_SMITHY_FFI_RESULT_CREATED) {
            throw Error("OpenKache returned an invalid NAMESPACE_OPEN outcome");
        }
        return {
            decode_namespace_descriptor(operation.payload),
            operation.kind == OPENKACHE_SMITHY_FFI_RESULT_CREATED,
        };
    }

    /// Replaces a namespace policy using its optimistic revision.
    Namespace_Descriptor namespace_update_policy(
        std::uint64_t namespace_id,
        std::uint64_t expected_revision,
        const Namespace_Policy& policy) const {
        const auto [flags, ttl_ms] = namespace_policy_wire(policy);
        auto* result = openkache_client_namespace_update_policy(
            client_,
            namespace_id,
            expected_revision,
            flags,
            ttl_ms);
        const auto operation = take_result(result);
        if (operation.kind != OPENKACHE_SMITHY_FFI_RESULT_VALUE) {
            throw Error("OpenKache returned an invalid NAMESPACE_UPDATE_POLICY outcome");
        }
        return decode_namespace_descriptor(operation.payload);
    }

    /// Deletes an empty namespace using its optimistic revision.
    void namespace_delete(
        std::uint64_t namespace_id,
        std::uint64_t expected_revision) const {
        auto* result = openkache_client_namespace_delete(
            client_,
            namespace_id,
            expected_revision);
        const auto operation = take_result(result);
        if (operation.kind != OPENKACHE_SMITHY_FFI_RESULT_OK) {
            throw Error("OpenKache returned an invalid NAMESPACE_DELETE outcome");
        }
    }

private:
    static Client connect_result(openkache_client_result_t* result) {
        if (result == nullptr) {
            throw Error("OpenKache connect returned a null result");
        }
        const auto kind = result_kind(result);
        if (kind != OPENKACHE_SMITHY_FFI_RESULT_CONNECTED) {
            const auto message = result_payload(result);
            openkache_client_result_free(result);
            throw Error(message.empty() ? "OpenKache connection failed" : message);
        }
        openkache_client_t* client = openkache_client_result_take_client(result);
        openkache_client_result_free(result);
        if (client == nullptr) {
            throw Error("OpenKache connect returned no client handle");
        }
        return Client(client);
    }

    struct Operation_Result {
        std::uint32_t kind;
        Bytes payload;
    };

    static std::optional<Bytes> get_outcome(
        Operation_Result result,
        const char* operation) {
        if (result.kind == OPENKACHE_SMITHY_FFI_RESULT_NOT_FOUND) {
            return std::nullopt;
        }
        if (result.kind != OPENKACHE_SMITHY_FFI_RESULT_VALUE) {
            throw Error(
                std::string("OpenKache returned an invalid ") + operation +
                " outcome");
        }
        return std::move(result.payload);
    }

    static Set_Outcome set_outcome(
        const Operation_Result& result,
        const char* operation) {
        switch (result.kind) {
        case OPENKACHE_SMITHY_FFI_RESULT_CREATED:
            return Set_Outcome::Created;
        case OPENKACHE_SMITHY_FFI_RESULT_REPLACED:
            return Set_Outcome::Replaced;
        case OPENKACHE_SMITHY_FFI_RESULT_NOT_STORED:
            return Set_Outcome::Not_Stored;
        default:
            throw Error(
                std::string("OpenKache returned an invalid ") + operation +
                " outcome");
        }
    }

    static bool delete_outcome(const Operation_Result& result) {
        if (result.kind == OPENKACHE_SMITHY_FFI_RESULT_DELETED) {
            return true;
        }
        if (result.kind == OPENKACHE_SMITHY_FFI_RESULT_NOT_DELETED) {
            return false;
        }
        throw Error("OpenKache returned an invalid DELETE outcome");
    }

    static std::span<const Byte> as_bytes(std::string_view value) noexcept {
        return {
            reinterpret_cast<const Byte*>(value.data()),
            value.size(),
        };
    }

    static std::pair<Byte, std::uint64_t> namespace_policy_wire(
        const Namespace_Policy& policy) {
        Byte flags = OPENKACHE_SMITHY_POLICY_NO_EXPIRY;
        std::uint64_t ttl_ms = policy.default_ttl_ms.value_or(0);
        switch (policy.default_expiration) {
        case Namespace_Expiration_Default::No_Expiry:
            if (policy.default_ttl_ms.has_value()) {
                throw Error("namespace No_Expiry policy must not contain a TTL");
            }
            break;
        case Namespace_Expiration_Default::Fixed_Ttl:
            if (!policy.default_ttl_ms.has_value() || ttl_ms == 0) {
                throw Error("namespace Fixed_Ttl policy requires a positive TTL");
            }
            flags = OPENKACHE_SMITHY_POLICY_FIXED_TTL;
            break;
        default:
            throw Error("OpenKache namespace expiration policy is not supported");
        }
        if (policy.expiration_override == Namespace_Override_Policy::Allowed) {
            flags |= OPENKACHE_SMITHY_POLICY_EXPIRATION_OVERRIDE;
        }
        switch (policy.default_eviction) {
        case Namespace_Eviction_Default::Evictable:
            break;
        case Namespace_Eviction_Default::Eviction_Protected:
            flags |= OPENKACHE_SMITHY_POLICY_EVICTION_PROTECTED;
            break;
        default:
            throw Error("OpenKache namespace eviction policy is not supported");
        }
        if (policy.eviction_override == Namespace_Override_Policy::Allowed) {
            flags |= OPENKACHE_SMITHY_POLICY_EVICTION_OVERRIDE;
        }
        return {flags, ttl_ms};
    }

    static Namespace_Descriptor decode_namespace_descriptor(const Bytes& payload) {
        openkache_client_namespace_descriptor_t decoded{};
        const auto* data = payload.empty() ? nullptr : payload.data();
        const auto status = openkache_client_namespace_descriptor_decode(
            data,
            payload.size(),
            &decoded);
        if (status != OPENKACHE_SMITHY_FFI_NAMESPACE_DESCRIPTOR_DECODE_OK) {
            throw Error("OpenKache returned an invalid namespace descriptor");
        }
        Namespace_Policy policy;
        policy.default_expiration = decoded.default_expiration
            == OPENKACHE_SMITHY_FFI_NAMESPACE_DEFAULT_EXPIRATION_FIXED_TTL
            ? Namespace_Expiration_Default::Fixed_Ttl
            : Namespace_Expiration_Default::No_Expiry;
        if (policy.default_expiration == Namespace_Expiration_Default::Fixed_Ttl) {
            policy.default_ttl_ms = decoded.default_ttl_ms;
        }
        policy.expiration_override = decoded.expiration_override
            == OPENKACHE_SMITHY_FFI_NAMESPACE_OVERRIDE_ALLOWED
            ? Namespace_Override_Policy::Allowed
            : Namespace_Override_Policy::Disallowed;
        policy.default_eviction = decoded.default_eviction
            == OPENKACHE_SMITHY_FFI_NAMESPACE_DEFAULT_EVICTION_PROTECTED
            ? Namespace_Eviction_Default::Eviction_Protected
            : Namespace_Eviction_Default::Evictable;
        policy.eviction_override = decoded.eviction_override
            == OPENKACHE_SMITHY_FFI_NAMESPACE_OVERRIDE_ALLOWED
            ? Namespace_Override_Policy::Allowed
            : Namespace_Override_Policy::Disallowed;
        return {decoded.namespace_id, decoded.revision, policy};
    }

    static std::uint32_t result_kind(const openkache_client_result_t* result) noexcept {
        return openkache_client_result_kind(result);
    }

    static std::string result_payload(const openkache_client_result_t* result) {
        const auto length = openkache_client_result_data_length(result);
        const auto* data = openkache_client_result_data(result);
        if (length == 0) {
            return {};
        }
        if (data == nullptr) {
            return "OpenKache returned a null payload";
        }
        return {
            reinterpret_cast<const char*>(data),
            length,
        };
    }

    static Operation_Result take_result(openkache_client_result_t* result) {
        if (result == nullptr) {
            throw Error("OpenKache operation returned a null result");
        }
        const auto kind = result_kind(result);
        if (kind == OPENKACHE_SMITHY_FFI_RESULT_ERROR) {
            const auto message = result_payload(result);
            openkache_client_result_free(result);
            throw Error(message.empty() ? "OpenKache operation failed" : message);
        }
        if (kind == OPENKACHE_SMITHY_FFI_RESULT_UNKNOWN_MUTATION) {
            const auto message = result_payload(result);
            openkache_client_result_free(result);
            throw Unknown_Mutation_Error(
                message.empty()
                    ? "OpenKache mutation outcome is unknown after cancellation"
                    : message);
        }
        if (kind == OPENKACHE_SMITHY_FFI_RESULT_CANCELED) {
            const auto message = result_payload(result);
            openkache_client_result_free(result);
            throw Canceled_Error(
                message.empty()
                    ? "OpenKache request was canceled before admission"
                    : message);
        }
        const auto length = openkache_client_result_data_length(result);
        const auto* data = openkache_client_result_data(result);
        Bytes payload;
        if (length != 0) {
            if (data == nullptr) {
                openkache_client_result_free(result);
                throw Error("OpenKache returned a null payload");
            }
            payload.assign(data, data + length);
        }
        openkache_client_result_free(result);
        return {kind, std::move(payload)};
    }

    struct Request_Guard {
        openkache_client_request_t* request;
        bool result_consumed = false;

        /// Publish cancellation before releasing an unconsumed request.
        ///
        /// C++ methods currently wait synchronously, so this is normally only
        /// exercised when an exception interrupts the wait/decoding path. It
        /// still closes the native admission boundary before `request_free`
        /// can discard a pending result.
        ~Request_Guard() noexcept {
            if (request != nullptr && !result_consumed) {
                (void)openkache_client_request_cancel(request);
            }
            openkache_client_request_free(request);
        }

        void mark_result_consumed() noexcept {
            result_consumed = true;
        }
    };

    static Operation_Result await_request(openkache_client_request_t* request) {
        if (request == nullptr) {
            throw Error("OpenKache operation returned a null request");
        }
        Request_Guard guard{request};
        while (openkache_client_request_poll(request)
            == OPENKACHE_SMITHY_FFI_REQUEST_STATE_PENDING) {
            std::this_thread::yield();
        }
        auto* result = openkache_client_request_wait(request, 0);
        guard.mark_result_consumed();
        return take_result(result);
    }

    static std::optional<std::tuple<std::uint32_t, Byte, std::uint64_t>> legacy_options(
        const Set_Options& options) {
        std::uint32_t condition;
        switch (options.condition) {
        case Set_Condition::Any:
            condition = OPENKACHE_SMITHY_FFI_SET_CONDITION_ANY;
            break;
        case Set_Condition::If_Absent:
            condition = OPENKACHE_SMITHY_FFI_SET_CONDITION_IF_ABSENT;
            break;
        case Set_Condition::If_Present:
            condition = OPENKACHE_SMITHY_FFI_SET_CONDITION_IF_PRESENT;
            break;
        default:
            return std::nullopt;
        }
        if (options.eviction_mode.has_value()
            && *options.eviction_mode != Eviction_Mode::Inherit) {
            return std::nullopt;
        }
        const auto expiration = options.expiration_mode.value_or(
            options.ttl_ms.has_value()
                ? Expiration_Mode::Explicit_Ttl
                : Expiration_Mode::Inherit);
        switch (expiration) {
        case Expiration_Mode::Inherit:
            if (options.ttl_ms.has_value()) {
                return std::nullopt;
            }
            return std::tuple{condition, Byte{0}, std::uint64_t{0}};
        case Expiration_Mode::Explicit_Ttl:
            if (!options.ttl_ms.has_value() || *options.ttl_ms == 0) {
                return std::nullopt;
            }
            return std::tuple{condition, Byte{1}, *options.ttl_ms};
        case Expiration_Mode::No_Expiry:
            return std::nullopt;
        default:
            return std::nullopt;
        }
    }

    Operation_Result execute(
        std::uint32_t operation,
        std::uint32_t key_spec,
        std::span<const Byte> key,
        std::span<const Byte> value,
        Set_Options options,
        bool raw = false) const {
        if (client_ == nullptr) {
            throw Error("OpenKache client is closed");
        }
        if (raw && key.size() > OPENKACHE_SMITHY_ITEM_ID_BYTES) {
            throw Error(
                "item ID must contain at most "
                + std::to_string(OPENKACHE_SMITHY_ITEM_ID_BYTES)
                + " bytes");
        }
        const auto [set_flags, ttl_ms] = wire_options(options);
        const auto* key_data = key.empty() ? nullptr : key.data();
        const auto* value_data = value.empty() ? nullptr : value.data();
        if (raw) {
            if (const auto legacy = legacy_options(options)) {
                const auto [condition, ttl_enabled, legacy_ttl] = *legacy;
                return await_request(openkache_client_execute_raw_async(
                    client_,
                    operation,
                    key_data,
                    key.size(),
                    value_data,
                    value.size(),
                    condition,
                    ttl_enabled,
                    legacy_ttl));
            }
            // ABI v1 has no raw request handle for the complete policy flags.
            // This synchronous call is the safe completion boundary.
            return take_result(openkache_client_execute_raw_with_options(
                client_,
                operation,
                key_data,
                key.size(),
                value_data,
                value.size(),
                set_flags,
                ttl_ms));
        }
        return await_request(openkache_client_execute_with_options_async(
            client_,
            operation,
            key_spec,
            key_data,
            key.size(),
            value_data,
            value.size(),
            set_flags,
            ttl_ms));
    }

    static std::pair<Byte, std::uint64_t> wire_options(const Set_Options& options) {
        Byte flags = OPENKACHE_SMITHY_SET_CONDITION_ANY_BITS;
        switch (options.condition) {
        case Set_Condition::Any:
            break;
        case Set_Condition::If_Absent:
            flags |= OPENKACHE_SMITHY_SET_IF_ABSENT_BITS;
            break;
        case Set_Condition::If_Present:
            flags |= OPENKACHE_SMITHY_SET_IF_PRESENT_BITS;
            break;
        default:
            throw Error("OpenKache SET condition is not supported");
        }

        const auto ttl = options.ttl_ms.value_or(0);
        if (options.ttl_ms.has_value() && ttl == 0) {
            throw Error("OpenKache SET TTL must be greater than zero milliseconds");
        }
        const auto expiration = options.expiration_mode.value_or(
            options.ttl_ms.has_value()
                ? Expiration_Mode::Explicit_Ttl
                : Expiration_Mode::Inherit);
        switch (expiration) {
        case Expiration_Mode::Inherit:
            if (options.ttl_ms.has_value()) {
                throw Error(
                    "OpenKache SET TTL is only valid with Explicit_Ttl expiration");
            }
            flags |= OPENKACHE_SMITHY_SET_INHERIT_EXPIRATION_BITS;
            break;
        case Expiration_Mode::No_Expiry:
            if (options.ttl_ms.has_value()) {
                throw Error(
                    "OpenKache SET TTL is only valid with Explicit_Ttl expiration");
            }
            flags |= OPENKACHE_SMITHY_SET_NO_EXPIRY_BITS;
            break;
        case Expiration_Mode::Explicit_Ttl:
            if (!options.ttl_ms.has_value()) {
                throw Error("OpenKache SET Explicit_Ttl requires a positive TTL");
            }
            flags |= OPENKACHE_SMITHY_SET_EXPLICIT_TTL_BITS;
            break;
        default:
            throw Error("OpenKache SET expiration mode is not supported");
        }

        switch (options.eviction_mode.value_or(Eviction_Mode::Inherit)) {
        case Eviction_Mode::Inherit:
            flags |= OPENKACHE_SMITHY_SET_INHERIT_EVICTION_BITS;
            break;
        case Eviction_Mode::Evictable:
            flags |= OPENKACHE_SMITHY_SET_EVICTABLE_BITS;
            break;
        case Eviction_Mode::Eviction_Protected:
            flags |= OPENKACHE_SMITHY_SET_EVICTION_PROTECTED_BITS;
            break;
        default:
            throw Error("OpenKache SET eviction mode is not supported");
        }
        return {flags, ttl};
    }

    openkache_client_t* client_ = nullptr;
};

} // namespace openkache

#endif /* OPENKACHE_CLIENT_HPP */
