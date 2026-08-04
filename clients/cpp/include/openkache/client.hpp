#ifndef OPENKACHE_CLIENT_HPP
#define OPENKACHE_CLIENT_HPP

#include <array>
#include <cstdint>
#include <optional>
#include <span>
#include <stdexcept>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

#include <openkache/client.h>

namespace openkache {

using Byte = std::uint8_t;
using Bytes = std::vector<Byte>;

/// Native C++ exception carrying a core or argument failure.
class Error : public std::runtime_error {
public:
    explicit Error(const std::string& message)
        : std::runtime_error(message) {}
};

/// Atomic existence condition for one SET operation.
enum class Set_Condition : std::uint32_t {
    Any = OPENKACHE_CLIENT_SET_CONDITION_ANY,
    If_Absent = OPENKACHE_CLIENT_SET_CONDITION_IF_ABSENT,
    If_Present = OPENKACHE_CLIENT_SET_CONDITION_IF_PRESENT,
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

/// Authenticated-encryption profile for formatted values.
enum class Encryption : std::uint32_t {
    Compact = OPENKACHE_CLIENT_ENCRYPTION_COMPACT,
    Robust = OPENKACHE_CLIENT_ENCRYPTION_ROBUST,
};

/// Successful SET outcome.
enum class Set_Outcome {
    Created,
    Replaced,
    Not_Stored,
};

/// Best-effort state of the native connection worker.
enum class Connection_State : std::uint32_t {
    Connected = OPENKACHE_CLIENT_CONNECTION_CONNECTED,
    Reconnecting = OPENKACHE_CLIENT_CONNECTION_RECONNECTING,
    Disconnected = OPENKACHE_CLIENT_CONNECTION_DISCONNECTED,
    Closed = OPENKACHE_CLIENT_CONNECTION_CLOSED,
    Unknown = OPENKACHE_CLIENT_CONNECTION_UNKNOWN,
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
    /// Hostname or numeric address with a UDP port.
    std::string address;
    /// TLS certificate identity. Empty selects the host in `address`.
    std::string server_name;
    /// One DER certificate or PEM chain. Empty selects system trust roots.
    std::vector<Byte> certificate;
    std::array<Byte, OPENKACHE_CLIENT_DATA_PROTECTION_KEY_BYTES> data_protection_key{};
    bool compression_enabled = false;
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
        if (result == nullptr) {
            throw Error("OpenKache connect returned a null result");
        }

        const auto kind = result_kind(result);
        if (kind != OPENKACHE_CLIENT_RESULT_CONNECTED) {
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
        case OPENKACHE_CLIENT_CONNECTION_CONNECTED:
            return Connection_State::Connected;
        case OPENKACHE_CLIENT_CONNECTION_RECONNECTING:
            return Connection_State::Reconnecting;
        case OPENKACHE_CLIENT_CONNECTION_DISCONNECTED:
            return Connection_State::Disconnected;
        case OPENKACHE_CLIENT_CONNECTION_CLOSED:
            return Connection_State::Closed;
        default:
            return Connection_State::Unknown;
        }
    }

    /// Replaces a failed connection without replaying an operation.
    void reconnect() const {
        const auto result = execute(
            OPENKACHE_CLIENT_OPERATION_RECONNECT, {}, {}, Set_Options{});
        if (result.kind != OPENKACHE_CLIENT_RESULT_OK) {
            throw Error("OpenKache returned an invalid RECONNECT outcome");
        }
    }

    /// Verifies the connection.
    void ping() const {
        const auto result = execute(
            OPENKACHE_CLIENT_OPERATION_PING, {}, {}, Set_Options{});
        if (result.kind != OPENKACHE_CLIENT_RESULT_OK) {
            throw Error("OpenKache returned an invalid PING outcome");
        }
    }

    /// Retrieves an application-key value, or `std::nullopt` when absent.
    std::optional<Bytes> get(std::span<const Byte> key) const {
        return get_outcome(
            execute(OPENKACHE_CLIENT_OPERATION_GET, key, {}, Set_Options{}),
            "GET");
    }

    /// Convenience overload for textual application keys.
    std::optional<Bytes> get(std::string_view key) const {
        return get(as_bytes(key));
    }

    /// Stores an application-key value and returns the server outcome.
    Set_Outcome set(
        std::span<const Byte> key,
        std::span<const Byte> value,
        Set_Options options = {}) const {
        return set_outcome(
            execute(OPENKACHE_CLIENT_OPERATION_SET, key, value, options),
            "SET");
    }

    /// Convenience overload for textual keys and values.
    Set_Outcome set(
        std::string_view key,
        std::string_view value,
        Set_Options options = {}) const {
        return set(as_bytes(key), as_bytes(value), options);
    }

    /// Deletes an application-key value and reports whether it existed.
    bool remove(std::span<const Byte> key) const {
        return delete_outcome(
            execute(OPENKACHE_CLIENT_OPERATION_DELETE, key, {}, Set_Options{}));
    }

    /// Convenience overload for textual application keys.
    bool remove(std::string_view key) const {
        return remove(as_bytes(key));
    }

    /// Retrieves exact bytes for a fixed-size protocol item ID.
    std::optional<Bytes> get_raw(std::span<const Byte> item_id) const {
        return get_outcome(
            execute(
                OPENKACHE_CLIENT_OPERATION_GET,
                item_id,
                {},
                Set_Options{},
                true),
            "raw GET");
    }

    /// Stores exact bytes for a fixed-size protocol item ID without value protection.
    Set_Outcome set_raw(
        std::span<const Byte> item_id,
        std::span<const Byte> value,
        Set_Options options = {}) const {
        return set_outcome(
            execute(
                OPENKACHE_CLIENT_OPERATION_SET,
                item_id,
                value,
                options,
                true),
            "raw SET");
    }

    /// Deletes a fixed-size protocol item ID without application-key derivation.
    bool remove_raw(std::span<const Byte> item_id) const {
        return delete_outcome(
            execute(
                OPENKACHE_CLIENT_OPERATION_DELETE,
                item_id,
                {},
                Set_Options{},
                true));
    }

    /// Returns the server's JSON statistics document.
    std::string stats() const {
        const auto result = execute(
            OPENKACHE_CLIENT_OPERATION_STATS, {}, {}, Set_Options{});
        if (result.kind != OPENKACHE_CLIENT_RESULT_VALUE) {
            throw Error("OpenKache returned an invalid STATS outcome");
        }
        if (result.payload.empty()) {
            return {};
        }
        return std::string(
            reinterpret_cast<const char*>(result.payload.data()),
            result.payload.size());
    }

    /// Waits for the server durability barrier.
    void sync() const {
        const auto result = execute(
            OPENKACHE_CLIENT_OPERATION_SYNC, {}, {}, Set_Options{});
        if (result.kind != OPENKACHE_CLIENT_RESULT_OK) {
            throw Error("OpenKache returned an invalid SYNC outcome");
        }
    }

private:
    struct Operation_Result {
        std::uint32_t kind;
        Bytes payload;
    };

    static std::optional<Bytes> get_outcome(
        Operation_Result result,
        const char* operation) {
        if (result.kind == OPENKACHE_CLIENT_RESULT_NOT_FOUND) {
            return std::nullopt;
        }
        if (result.kind != OPENKACHE_CLIENT_RESULT_VALUE) {
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
        case OPENKACHE_CLIENT_RESULT_CREATED:
            return Set_Outcome::Created;
        case OPENKACHE_CLIENT_RESULT_REPLACED:
            return Set_Outcome::Replaced;
        case OPENKACHE_CLIENT_RESULT_NOT_STORED:
            return Set_Outcome::Not_Stored;
        default:
            throw Error(
                std::string("OpenKache returned an invalid ") + operation +
                " outcome");
        }
    }

    static bool delete_outcome(const Operation_Result& result) {
        if (result.kind == OPENKACHE_CLIENT_RESULT_DELETED) {
            return true;
        }
        if (result.kind == OPENKACHE_CLIENT_RESULT_NOT_DELETED) {
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
        if (kind == OPENKACHE_CLIENT_RESULT_ERROR) {
            const auto message = result_payload(result);
            openkache_client_result_free(result);
            throw Error(message.empty() ? "OpenKache operation failed" : message);
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

    Operation_Result execute(
        std::uint32_t operation,
        std::span<const Byte> key,
        std::span<const Byte> value,
        Set_Options options,
        bool raw = false) const {
        if (client_ == nullptr) {
            throw Error("OpenKache client is closed");
        }
        const auto [set_flags, ttl_ms] = wire_options(options);
        const auto* key_data = key.empty() ? nullptr : key.data();
        const auto* value_data = value.empty() ? nullptr : value.data();
        auto* result = raw
            ? openkache_client_execute_raw_with_options(
                  client_,
                  operation,
                  key_data,
                  key.size(),
                  value_data,
                  value.size(),
                  set_flags,
                  ttl_ms)
            : openkache_client_execute_with_options(
                  client_,
                  operation,
                  key_data,
                  key.size(),
                  value_data,
                  value.size(),
                  set_flags,
                  ttl_ms);
        return take_result(result);
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
