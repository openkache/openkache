#ifndef OPENKACHE_CLIENT_HPP
#define OPENKACHE_CLIENT_HPP

#include <cstdint>
#include <optional>
#include <span>
#include <stdexcept>
#include <string>
#include <string_view>
#include <utility>
#include <variant>

#include <openkache/client.h>
#include <openkache/value.hpp>

namespace openkache {

/// Native C++ exception carrying a validation, transport, or server failure.
class Error : public std::runtime_error {
public:
  explicit Error(std::string message)
      : std::runtime_error(std::move(message)) {}
};

/// A mutation whose response may have been lost after admission.
class Unknown_Mutation_Error : public Error {
public:
  explicit Unknown_Mutation_Error(std::string message)
      : Error(std::move(message)) {}
};

enum class Key_Kind {
  Integer,
  Text,
  Bytes,
};

/// One Gate 0 typed application key.
class Typed_Key {
public:
  explicit Typed_Key(std::int64_t value) : value_(value) {}
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
      return OPENKACHE_CLIENT_KEY_INTEGER;
    case Key_Kind::Text:
      return OPENKACHE_CLIENT_KEY_TEXT;
    case Key_Kind::Bytes:
      return OPENKACHE_CLIENT_KEY_BYTES;
    }
    return OPENKACHE_CLIENT_KEY_BYTES;
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
  limits.max_bytes = 1u << 20;
  const auto encoded = encode_structured_value(value, limits);
  if (encoded.empty() || encoded.size() > limits.max_bytes) {
    throw Error("canonical key exceeds the 1 MiB limit");
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

  /// Idempotently closes the connection and releases the native worker.
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
    if (result.kind == OPENKACHE_CLIENT_RESULT_NOT_FOUND) {
      return Get_Result::missing();
    }
    if (result.kind != OPENKACHE_CLIENT_RESULT_VALUE) {
      throw_result_error(result, "GET");
    }
    try {
      return Get_Result::found(decode_structured_value(result.payload));
    } catch (const Value_Error &error) {
      throw Error(std::string("GET value decoding failed: ") + error.what());
    }
  }

  Set_Outcome set(const Typed_Key &key, const Value &value) const {
    const auto canonical = canonical_key_bytes(key);
    const auto payload = encode_structured_value(value);
    const auto result = take_result(openkache_client_gate0_set(
        checked_client(), canonical.data(), canonical.size(), payload.data(),
        payload.size()));
    if (result.kind == OPENKACHE_CLIENT_RESULT_CREATED) {
      return Set_Outcome::Created;
    }
    if (result.kind == OPENKACHE_CLIENT_RESULT_REPLACED) {
      return Set_Outcome::Replaced;
    }
    throw_result_error(result, "SET");
    return Set_Outcome::Created;
  }

  Delete_Outcome remove(const Typed_Key &key) const {
    const auto logical = key.logical_bytes();
    const auto result = take_result(openkache_client_gate0_delete_value(
        checked_client(), key.key_spec(), logical.data(), logical.size()));
    if (result.kind == OPENKACHE_CLIENT_RESULT_DELETED) {
      return Delete_Outcome::Deleted;
    }
    if (result.kind == OPENKACHE_CLIENT_RESULT_NOT_DELETED ||
        result.kind == OPENKACHE_CLIENT_RESULT_NOT_FOUND) {
      return Delete_Outcome::NotFound;
    }
    throw_result_error(result, "DELETE");
    return Delete_Outcome::NotFound;
  }

  Get_Result get(std::int64_t key) const {
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

  Set_Outcome set(std::string_view key, const Value &value) const {
    return set(Typed_Key::text(key), value);
  }

  Set_Outcome set(std::span<const Byte> key, const Value &value) const {
    return set(Typed_Key::bytes(key), value);
  }

  Delete_Outcome remove(std::int64_t key) const {
    return remove(Typed_Key::integer(key));
  }

  Delete_Outcome remove(std::string_view key) const {
    return remove(Typed_Key::text(key));
  }

  Delete_Outcome remove(std::span<const Byte> key) const {
    return remove(Typed_Key::bytes(key));
  }

private:
  struct Native_Result {
    std::uint32_t kind = OPENKACHE_CLIENT_RESULT_ERROR;
    std::uint32_t status = OPENKACHE_CLIENT_STATUS_ERROR;
    std::uint32_t error_category = OPENKACHE_CLIENT_ERROR_INTERNAL;
    Bytes payload;
  };

  static Client connect_result(openkache_client_result_t *result) {
    if (result == nullptr) {
      throw Error("OpenKache connect returned a null result");
    }
    const auto kind = openkache_client_gate0_result_kind(result);
    if (kind != OPENKACHE_CLIENT_RESULT_CONNECTED) {
      const auto message = result_message(result);
      openkache_client_gate0_result_free(result);
      throw Error(message.empty() ? "OpenKache connection failed" : message);
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
    if (result.kind == OPENKACHE_CLIENT_RESULT_UNKNOWN_MUTATION ||
        result.error_category == OPENKACHE_CLIENT_ERROR_UNKNOWN_MUTATION) {
      throw Unknown_Mutation_Error(message);
    }
    throw Error(message);
  }

  openkache_client_t *client_ = nullptr;
};

} // namespace openkache

#endif /* OPENKACHE_CLIENT_HPP */
