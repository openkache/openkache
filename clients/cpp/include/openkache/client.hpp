#ifndef OPENKACHE_CLIENT_HPP
#define OPENKACHE_CLIENT_HPP

#include <cstdint>
#include <limits>
#include <optional>
#include <span>
#include <stdexcept>
#include <string>
#include <string_view>
#include <type_traits>
#include <utility>
#include <variant>

#include <openkache/client.h>
#include <openkache/value.hpp>

namespace openkache {

static_assert(OPENKACHE_CLIENT_GATE0_NAMESPACE_ID ==
                  OPENKACHE_SMITHY_GATE0_NAMESPACE_ID,
              "Gate 0 namespace identity drifted from Smithy");

/// Native C++ exception carrying a validation, transport, or server failure.
class Error : public std::runtime_error {
public:
  explicit Error(std::string message)
      : std::runtime_error(std::move(message)) {}
};

/// Stable native error categories projected by the shared C ABI.
enum class Error_Category : std::uint32_t {
  None = OPENKACHE_CLIENT_GATE0_ERROR_NONE,
  Invalid_Input = OPENKACHE_CLIENT_GATE0_ERROR_INVALID_INPUT,
  Configuration = OPENKACHE_CLIENT_GATE0_ERROR_CONFIGURATION,
  Timeout = OPENKACHE_CLIENT_GATE0_ERROR_TIMEOUT,
  Transport = OPENKACHE_CLIENT_GATE0_ERROR_TRANSPORT,
  Server = OPENKACHE_CLIENT_GATE0_ERROR_SERVER,
  Protocol = OPENKACHE_CLIENT_GATE0_ERROR_PROTOCOL,
  Value = OPENKACHE_CLIENT_GATE0_ERROR_VALUE,
  Key = OPENKACHE_CLIENT_GATE0_ERROR_KEY,
  Unknown_Mutation = OPENKACHE_CLIENT_GATE0_ERROR_UNKNOWN_MUTATION,
  Resource_Exhausted = OPENKACHE_CLIENT_GATE0_ERROR_RESOURCE_EXHAUSTED,
  Closed = OPENKACHE_CLIENT_GATE0_ERROR_CLOSED,
  Internal = OPENKACHE_CLIENT_GATE0_ERROR_INTERNAL,
};

/// Native failure with a stable shared-core error category.
class Native_Error : public Error {
public:
  Native_Error(std::string message, Error_Category category)
      : Error(std::move(message)), category_(category) {}

  Error_Category category() const noexcept { return category_; }

private:
  Error_Category category_;
};

/// A mutation whose response may have been lost after admission.
class Unknown_Mutation_Error : public Native_Error {
public:
  explicit Unknown_Mutation_Error(std::string message)
      : Native_Error(std::move(message), Error_Category::Unknown_Mutation) {}
};

enum class Key_Kind {
  Integer,
  Text,
  Bytes,
};

namespace detail {

template <typename T>
std::int64_t checked_i64_key(T value) {
  using Source = std::remove_cvref_t<T>;
  static_assert(detail::is_integer_v<Source>);
  static_assert(!std::is_same_v<Source, bool>);

  if constexpr (detail::is_signed_integer_v<Source>) {
    if constexpr (std::numeric_limits<Source>::digits >
                  std::numeric_limits<std::int64_t>::digits) {
      if (value < static_cast<Source>(std::numeric_limits<std::int64_t>::min()) ||
          value > static_cast<Source>(std::numeric_limits<std::int64_t>::max())) {
        throw Error("integer key is outside the signed i64 range");
      }
    }
  } else if constexpr (std::numeric_limits<Source>::digits >
                       std::numeric_limits<std::int64_t>::digits) {
    if (value > static_cast<Source>(std::numeric_limits<std::int64_t>::max())) {
      throw Error("integer key is outside the signed i64 range");
    }
  }
  return static_cast<std::int64_t>(value);
}

} // namespace detail

/// One Gate 0 typed application key.
class Typed_Key {
public:
  template <typename T>
    requires(std::is_same_v<std::remove_cvref_t<T>, bool>)
  explicit Typed_Key(T) = delete;

  template <typename T>
    requires(std::is_floating_point_v<std::remove_cvref_t<T>>)
  explicit Typed_Key(T) = delete;

  explicit Typed_Key(std::int64_t value) : value_(value) {}

  template <typename T>
    requires(
        detail::is_integer_v<T> &&
        !std::is_same_v<std::remove_cvref_t<T>, bool> &&
        !std::is_same_v<std::remove_cvref_t<T>, std::int64_t>)
  explicit Typed_Key(T value) : Typed_Key(detail::checked_i64_key(value)) {}
  explicit Typed_Key(const char *value)
      : Typed_Key(value == nullptr ? std::string_view{}
                                   : std::string_view(value)) {
    if (value == nullptr) {
      throw Error("text key pointer must not be null");
    }
  }
  explicit Typed_Key(std::string_view value) : value_(Text(value)) {
    detail::ensure_utf8(value);
  }
  explicit Typed_Key(Text value) : value_(std::move(value)) {
    detail::ensure_utf8(std::get<Text>(value_));
  }
  explicit Typed_Key(Bytes value) : value_(std::move(value)) {}
  explicit Typed_Key(std::span<const Byte> value)
      : value_(Bytes(value.begin(), value.end())) {}

  static Typed_Key integer(std::int64_t value) { return Typed_Key(value); }

  template <typename T>
    requires(
        detail::is_integer_v<T> &&
        !std::is_same_v<std::remove_cvref_t<T>, bool> &&
        !std::is_same_v<std::remove_cvref_t<T>, std::int64_t>)
  static Typed_Key integer(T value) {
    return Typed_Key(value);
  }

  template <typename T>
    requires(std::is_same_v<std::remove_cvref_t<T>, bool>)
  static Typed_Key integer(T) = delete;

  template <typename T>
    requires(std::is_floating_point_v<std::remove_cvref_t<T>>)
  static Typed_Key integer(T) = delete;

  static Typed_Key text(std::string_view value) { return Typed_Key(value); }

  static Typed_Key bytes(Bytes value) { return Typed_Key(std::move(value)); }

  static Typed_Key bytes(std::span<const Byte> value) {
    return Typed_Key(value);
  }

  Key_Kind kind() const noexcept {
    switch (value_.index()) {
    case 0:
      return Key_Kind::Integer;
    case 1:
      return Key_Kind::Text;
    default:
      return Key_Kind::Bytes;
    }
  }

  std::int64_t integer() const {
    if (kind() != Key_Kind::Integer) {
      throw Error("key is not an Integer");
    }
    return std::get<std::int64_t>(value_);
  }

  const Text &text() const {
    if (kind() != Key_Kind::Text) {
      throw Error("key is not Text");
    }
    return std::get<Text>(value_);
  }

  const Bytes &bytes() const {
    if (kind() != Key_Kind::Bytes) {
      throw Error("key is not Bytes");
    }
    return std::get<Bytes>(value_);
  }

  std::uint32_t key_spec() const noexcept {
    switch (kind()) {
    case Key_Kind::Integer:
      return OPENKACHE_CLIENT_GATE0_KEY_INTEGER;
    case Key_Kind::Text:
      return OPENKACHE_CLIENT_GATE0_KEY_TEXT;
    case Key_Kind::Bytes:
      return OPENKACHE_CLIENT_GATE0_KEY_BYTES;
    }
    return OPENKACHE_CLIENT_GATE0_KEY_BYTES;
  }

  /// Returns the logical bytes used by the typed delete ABI.
  Bytes logical_bytes() const {
    switch (kind()) {
    case Key_Kind::Integer: {
      const auto decimal = std::to_string(integer());
      return Bytes(decimal.begin(), decimal.end());
    }
    case Key_Kind::Text: {
      const auto &value = text();
      return Bytes(reinterpret_cast<const Byte *>(value.data()),
                   reinterpret_cast<const Byte *>(value.data()) + value.size());
    }
    case Key_Kind::Bytes:
      return bytes();
    }
    return {};
  }

private:
  std::variant<std::int64_t, Text, Bytes> value_;
};

using Key = Typed_Key;
using TypedKey = Typed_Key;

namespace detail {

inline Bytes canonical_key_bytes(const Typed_Key &key) {
  Value value = [&]() {
    switch (key.kind()) {
    case Key_Kind::Integer:
      return Value::integer(key.integer());
    case Key_Kind::Text:
      return Value::text(key.text());
    case Key_Kind::Bytes:
      return Value::bytes(key.bytes());
    }
    return Value::undefined();
  }();
  Value_Limits limits;
  limits.max_bytes = OPENKACHE_SMITHY_MAX_CANONICAL_KEY_BYTES;
  const auto encoded = encode_structured_value(value, limits);
  if (encoded.empty() || encoded.size() > limits.max_bytes) {
    throw Error("canonical key exceeds " + std::to_string(limits.max_bytes) +
                " bytes");
  }
  return encoded;
}

} // namespace detail

/// Returns one deterministic-CBOR key item for the supplied typed key.
inline Bytes canonical_key_bytes(const Typed_Key &key) {
  return detail::canonical_key_bytes(key);
}

enum class Get_Result_Kind {
  Missing,
  Found,
};

/// Tagged GET result; Missing is distinct from Found(Null) and
/// Found(Undefined).
class Get_Result {
public:
  static Get_Result missing() {
    return Get_Result(Get_Result_Kind::Missing, std::nullopt);
  }

  static Get_Result found(Value value) {
    return Get_Result(Get_Result_Kind::Found, std::move(value));
  }

  Get_Result_Kind kind() const noexcept { return kind_; }

  bool is_missing() const noexcept { return kind_ == Get_Result_Kind::Missing; }

  bool is_found() const noexcept { return kind_ == Get_Result_Kind::Found; }

  const Value &value() const {
    if (!value_) {
      throw Error("GET result is Missing");
    }
    return *value_;
  }

  Value take_value() {
    if (!value_) {
      throw Error("GET result is Missing");
    }
    Value result = std::move(*value_);
    value_.reset();
    kind_ = Get_Result_Kind::Missing;
    return result;
  }

private:
  Get_Result(Get_Result_Kind kind, std::optional<Value> value)
      : kind_(kind), value_(std::move(value)) {}

  Get_Result_Kind kind_;
  std::optional<Value> value_;
};

using GetResult = Get_Result;

enum class Set_Outcome {
  Created,
  Replaced,
};

enum class Delete_Outcome {
  Deleted,
  NotFound,
};

using Remove_Outcome = Delete_Outcome;

/// The sole Gate 0 connection input.  Transport, trust, and timing are fixed.
struct Connect_Options {
  std::string address;

  Connect_Options() = default;
  explicit Connect_Options(std::string_view value) : address(value) {}
};

/// Synchronous RAII C++20 adapter for the five-operation Gate 0 facade.
///
/// Call `close()` explicitly when the client is no longer needed. The
/// destructor is only a best-effort fallback for abandoned owners: it cannot
/// report failures and its execution timing follows normal C++ destruction.
class Client {
public:
  Client() noexcept = default;

  explicit Client(openkache_client_t *client) noexcept : client_(client) {}

  Client(const Client &) = delete;
  Client &operator=(const Client &) = delete;

  Client(Client &&other) noexcept
      : client_(std::exchange(other.client_, nullptr)) {}

  Client &operator=(Client &&other) noexcept {
    if (this != &other) {
      close();
      client_ = std::exchange(other.client_, nullptr);
    }
    return *this;
  }

  ~Client() noexcept { close(); }

  /// Connects with the fixed TLS 1.3 DevelopmentTrust profile.
  ///
  /// DevelopmentTrust keeps TLS encryption and the `openkache/1` ALPN but
  /// disables certificate and hostname verification.  It is development
  /// only — do not use this trust profile in production.
  static Client connect(const Connect_Options &options) {
    validate_native_abi();
    if (options.address.empty()) {
      throw Error("OpenKache endpoint must not be empty");
    }
    auto *result = openkache_client_gate0_connect(
        reinterpret_cast<const Byte *>(options.address.data()),
        options.address.size());
    return connect_result(result);
  }

  static Client connect(std::string_view address) {
    return connect(Connect_Options(address));
  }

  explicit operator bool() const noexcept { return client_ != nullptr; }

  /// Explicitly closes the connection and releases the native worker.
  ///
  /// This is the normative lifecycle boundary. The operation is idempotent;
  /// callers must not race it with `get`, `set`, or `remove`. The destructor
  /// invokes the same no-throw path only as a best-effort fallback when the
  /// caller does not close explicitly.
  void close() noexcept {
    if (client_ != nullptr) {
      openkache_client_gate0_close(client_);
      client_ = nullptr;
    }
  }

  Get_Result get(const Typed_Key &key) const {
    const auto canonical = canonical_key_bytes(key);
    auto result = take_result(openkache_client_gate0_get(
        checked_client(), canonical.data(), canonical.size()));
    if (result.kind == OPENKACHE_CLIENT_GATE0_RESULT_NOT_FOUND) {
      return Get_Result::missing();
    }
    if (result.kind != OPENKACHE_CLIENT_GATE0_RESULT_VALUE) {
      throw_result_error(result, "GET");
    }
    try {
      return Get_Result::found(decode_structured_value(result.payload));
    } catch (const Value_Error &error) {
      throw Value_Error(std::string("GET value decoding failed: ") + error.what(),
                        error.kind(), error.resource(), error.limit(),
                        error.actual());
    }
  }

  Set_Outcome set(const Typed_Key &key, const Value &value) const {
    const auto canonical = canonical_key_bytes(key);
    const auto payload = encode_structured_value(value);
    const auto result = take_result(openkache_client_gate0_set(
        checked_client(), canonical.data(), canonical.size(), payload.data(),
        payload.size()));
    if (result.kind == OPENKACHE_CLIENT_GATE0_RESULT_CREATED) {
      return Set_Outcome::Created;
    }
    if (result.kind == OPENKACHE_CLIENT_GATE0_RESULT_REPLACED) {
      return Set_Outcome::Replaced;
    }
    throw_result_error(result, "SET");
    return Set_Outcome::Created;
  }

  Delete_Outcome remove(const Typed_Key &key) const {
    const auto canonical = canonical_key_bytes(key);
    const auto result = take_result(openkache_client_gate0_delete_value(
        checked_client(), canonical.data(), canonical.size()));
    if (result.kind == OPENKACHE_CLIENT_GATE0_RESULT_DELETED) {
      return Delete_Outcome::Deleted;
    }
    if (result.kind == OPENKACHE_CLIENT_GATE0_RESULT_NOT_DELETED ||
        result.kind == OPENKACHE_CLIENT_GATE0_RESULT_NOT_FOUND) {
      return Delete_Outcome::NotFound;
    }
    throw_result_error(result, "DELETE");
    return Delete_Outcome::NotFound;
  }

  template <typename T>
    requires(std::is_same_v<std::remove_cvref_t<T>, bool>)
  Get_Result get(T) const = delete;

  template <typename T>
    requires(std::is_floating_point_v<std::remove_cvref_t<T>>)
  Get_Result get(T) const = delete;

  Get_Result get(std::int64_t key) const {
    return get(Typed_Key::integer(key));
  }

  template <typename T>
    requires(
        detail::is_integer_v<T> &&
        !std::is_same_v<std::remove_cvref_t<T>, bool> &&
        !std::is_same_v<std::remove_cvref_t<T>, std::int64_t>)
  Get_Result get(T key) const {
    return get(Typed_Key::integer(key));
  }

  Get_Result get(std::string_view key) const {
    return get(Typed_Key::text(key));
  }

  Get_Result get(std::span<const Byte> key) const {
    return get(Typed_Key::bytes(key));
  }

  Set_Outcome set(std::int64_t key, const Value &value) const {
    return set(Typed_Key::integer(key), value);
  }

  template <typename T>
    requires(
        detail::is_integer_v<T> &&
        !std::is_same_v<std::remove_cvref_t<T>, bool> &&
        !std::is_same_v<std::remove_cvref_t<T>, std::int64_t>)
  Set_Outcome set(T key, const Value &value) const {
    return set(Typed_Key::integer(key), value);
  }

  Set_Outcome set(std::string_view key, const Value &value) const {
    return set(Typed_Key::text(key), value);
  }

  Set_Outcome set(std::span<const Byte> key, const Value &value) const {
    return set(Typed_Key::bytes(key), value);
  }

  Delete_Outcome remove(std::int64_t key) const {
    return remove(Typed_Key::integer(key));
  }

  template <typename T>
    requires(
        detail::is_integer_v<T> &&
        !std::is_same_v<std::remove_cvref_t<T>, bool> &&
        !std::is_same_v<std::remove_cvref_t<T>, std::int64_t>)
  Delete_Outcome remove(T key) const {
    return remove(Typed_Key::integer(key));
  }

  Delete_Outcome remove(std::string_view key) const {
    return remove(Typed_Key::text(key));
  }

  Delete_Outcome remove(std::span<const Byte> key) const {
    return remove(Typed_Key::bytes(key));
  }

  template <typename T>
    requires(std::is_same_v<std::remove_cvref_t<T>, bool>)
  Set_Outcome set(T, const Value &) const = delete;

  template <typename T>
    requires(std::is_floating_point_v<std::remove_cvref_t<T>>)
  Set_Outcome set(T, const Value &) const = delete;

  template <typename T>
    requires(std::is_same_v<std::remove_cvref_t<T>, bool>)
  Delete_Outcome remove(T) const = delete;

  template <typename T>
    requires(std::is_floating_point_v<std::remove_cvref_t<T>>)
  Delete_Outcome remove(T) const = delete;

private:
  struct Native_Result {
    std::uint32_t kind = OPENKACHE_CLIENT_GATE0_RESULT_ERROR;
    std::uint32_t status = OPENKACHE_CLIENT_GATE0_STATUS_ERROR;
    std::uint32_t error_category = OPENKACHE_CLIENT_GATE0_ERROR_INTERNAL;
    Bytes payload;
  };

  static void validate_native_abi() {
    const auto actual = openkache_client_abi_version();
    if (actual != OPENKACHE_CLIENT_ABI_VERSION) {
      throw Error(
          "OpenKache native ABI version " + std::to_string(actual) +
          " does not match generated contract version " +
          std::to_string(OPENKACHE_CLIENT_ABI_VERSION));
    }
  }

  static Client connect_result(openkache_client_result_t *result) {
    if (result == nullptr) {
      throw Error("OpenKache connect returned a null result");
    }
    const auto kind = openkache_client_gate0_result_kind(result);
    if (kind != OPENKACHE_CLIENT_GATE0_RESULT_CONNECTED) {
      const auto message = result_message(result);
      const auto category =
          openkache_client_gate0_result_error_category(result);
      openkache_client_gate0_result_free(result);
      const auto error_message =
          message.empty() ? "OpenKache connection failed" : message;
      if (kind == OPENKACHE_CLIENT_GATE0_RESULT_UNKNOWN_MUTATION ||
          category == OPENKACHE_CLIENT_GATE0_ERROR_UNKNOWN_MUTATION) {
        throw Unknown_Mutation_Error(error_message);
      }
      throw Native_Error(error_message,
                         static_cast<Error_Category>(category));
    }
    auto *client = openkache_client_gate0_result_take_client(result);
    openkache_client_gate0_result_free(result);
    if (client == nullptr) {
      throw Error("OpenKache connect returned no client handle");
    }
    return Client(client);
  }

  const openkache_client_t *checked_client() const {
    if (client_ == nullptr) {
      throw Error("OpenKache client is closed");
    }
    return client_;
  }

  static std::string result_message(const openkache_client_result_t *result) {
    const auto length = openkache_client_gate0_result_data_length(result);
    const auto *data = openkache_client_gate0_result_data(result);
    if (length == 0)
      return {};
    if (data == nullptr)
      return "OpenKache returned a null error payload";
    return std::string(reinterpret_cast<const char *>(data), length);
  }

  static Native_Result take_result(openkache_client_result_t *result) {
    if (result == nullptr) {
      throw Error("OpenKache operation returned a null result");
    }
    Native_Result output;
    output.kind = openkache_client_gate0_result_kind(result);
    output.status = openkache_client_gate0_result_status(result);
    output.error_category =
        openkache_client_gate0_result_error_category(result);
    const auto length = openkache_client_gate0_result_data_length(result);
    const auto *data = openkache_client_gate0_result_data(result);
    if (length != 0) {
      if (data == nullptr) {
        openkache_client_gate0_result_free(result);
        throw Error("OpenKache returned a null result payload");
      }
      output.payload.assign(data, data + length);
    }
    openkache_client_gate0_result_free(result);
    return output;
  }

  [[noreturn]] static void throw_result_error(const Native_Result &result,
                                              const char *operation) {
    const std::string message =
        result.payload.empty()
            ? std::string("OpenKache ") + operation + " failed"
            : std::string(reinterpret_cast<const char *>(result.payload.data()),
                          result.payload.size());
    if (result.kind == OPENKACHE_CLIENT_GATE0_RESULT_UNKNOWN_MUTATION ||
        result.error_category ==
            OPENKACHE_CLIENT_GATE0_ERROR_UNKNOWN_MUTATION) {
      throw Unknown_Mutation_Error(message);
    }
    throw Native_Error(message,
                       static_cast<Error_Category>(result.error_category));
  }

  openkache_client_t *client_ = nullptr;
};

} // namespace openkache

#endif /* OPENKACHE_CLIENT_HPP */
