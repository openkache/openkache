#ifndef OPENKACHE_VALUE_HPP
#define OPENKACHE_VALUE_HPP

/*
 * Lossless OpenKache Value model and StructuredValue-CBOR-v1 codec.
 *
 * The implementation is deliberately header-only so the C++ package remains
 * an adapter over the shared native core.  It does not inspect JSON or infer
 * a value format from bytes: callers always provide one Value and the codec
 * emits or accepts exactly one complete CBOR item.
 */

#include <algorithm>
#include <array>
#include <charconv>
#include <cstdint>
#include <cstring>
#include <initializer_list>
#include <limits>
#include <memory>
#include <new>
#include <optional>
#include <span>
#include <stdexcept>
#include <string>
#include <string_view>
#include <type_traits>
#include <unordered_map>
#include <utility>
#include <variant>
#include <vector>

#include <openkache/smithy_contract.h>

namespace openkache {

using Byte = std::uint8_t;
using Bytes = std::vector<Byte>;
using Text = std::string;

enum class Value_Error_Kind {
  Resource_Limit,
  Truncated,
  Trailing_Bytes,
  Invalid_Encoding,
  Unsupported_Type,
  Invalid_Utf8,
  Invalid_Integer,
  Non_Scalar_Key,
  Duplicate_Key,
  Allocation,
};

enum class Value_Resource {
  Bytes,
  Depth,
  Items,
  Integer_Bytes,
};

/// Mathematical sign used when constructing an arbitrary-precision integer.
enum class Sign {
  Positive,
  Negative,
};

/// Error raised by value construction, validation, encoding, or decoding.
class Value_Error : public std::runtime_error {
public:
  explicit Value_Error(
      std::string message,
      Value_Error_Kind kind = Value_Error_Kind::Invalid_Encoding,
      Value_Resource resource = Value_Resource::Bytes, std::size_t limit = 0,
      std::size_t actual = 0)
      : std::runtime_error(std::move(message)), kind_(kind),
        resource_(resource), limit_(limit), actual_(actual) {}

  Value_Error_Kind kind() const noexcept { return kind_; }

  Value_Error_Kind category() const noexcept { return kind_; }

  Value_Resource resource() const noexcept { return resource_; }

  std::size_t limit() const noexcept { return limit_; }

  std::size_t actual() const noexcept { return actual_; }

private:
  Value_Error_Kind kind_;
  Value_Resource resource_;
  std::size_t limit_;
  std::size_t actual_;
};

/// Bounded work budget shared by the structured encoder and decoder.
struct Value_Limits {
  std::size_t max_bytes = OPENKACHE_SMITHY_MAX_VALUE_BYTES;
  std::size_t max_depth = 128;
  std::size_t max_items = 1'000'000;
  std::size_t max_integer_bytes = 1u << 20;
};

/// Hard upper bound for caller-selected traversal depth.
// The decoder is deliberately bounded below the platform's typical native
// stack budget.  A caller can still choose a lower limit per operation, while
// this hard ceiling prevents a user-controlled limit from turning the
// recursive value destructor/parser into an unbounded stack walk.
inline constexpr std::size_t MAX_ALLOWED_VALUE_DEPTH = 1'024;

struct Undefined_Value final {};
struct Null_Value final {};

/// Exact arbitrary-precision signed integer in normalized sign/magnitude form.
class Integer {
public:
  Integer() = default;

  explicit Integer(std::int64_t value) {
    if (value < 0) {
      negative_ = true;
      append_u64(magnitude_, value == std::numeric_limits<std::int64_t>::min()
                                 ? (std::uint64_t{1} << 63)
                                 : static_cast<std::uint64_t>(-value));
    } else {
      append_u64(magnitude_, static_cast<std::uint64_t>(value));
    }
  }

  explicit Integer(std::string_view decimal) { *this = from_decimal(decimal); }

  static Integer from_i64(std::int64_t value) { return Integer(value); }

  static Integer from_u64(std::uint64_t value) {
    Integer result;
    append_u64(result.magnitude_, value);
    return result;
  }

#if defined(__SIZEOF_INT128__)
  // Keep the optional 128-bit convenience constructors available on
  // compilers that provide them without exposing a non-standard type spelling
  // to strict `-Wpedantic -Werror` consumers.
  __extension__ using unsigned_int128 = unsigned __int128;
  __extension__ using signed_int128 = __int128;

  static Integer from_u128(unsigned_int128 value) {
    Integer result;
    if (value == 0) {
      return result;
    }
    std::array<Byte, 16> bytes{};
    for (std::size_t index = bytes.size(); index != 0; --index) {
      bytes[index - 1] = static_cast<Byte>(value & 0xffu);
      value >>= 8u;
    }
    const auto first = first_nonzero(bytes);
    result.magnitude_.assign(bytes.begin() + static_cast<std::ptrdiff_t>(first),
                             bytes.end());
    return result;
  }

  static Integer from_i128(signed_int128 value) {
    if (value >= 0) {
      return from_u128(static_cast<unsigned_int128>(value));
    }
    // Converting through unsigned arithmetic avoids overflowing for the
    // minimum representable signed value.
    const auto magnitude = static_cast<unsigned_int128>(-(value + 1)) + 1u;
    auto result = from_u128(magnitude);
    result.negative_ = !result.magnitude_.empty();
    return result;
  }
#endif

  static Integer from_sign_and_magnitude(bool negative,
                                         std::span<const Byte> magnitude) {
    Integer result;
    const auto first = first_nonzero(magnitude);
    if (first != magnitude.size()) {
      result.negative_ = negative;
      result.magnitude_.assign(magnitude.begin() +
                                   static_cast<std::ptrdiff_t>(first),
                               magnitude.end());
    }
    return result;
  }

  static Integer from_magnitude_be(bool negative, const Bytes &magnitude) {
    return from_sign_and_magnitude(negative, magnitude);
  }

  static Integer from_magnitude_be(Sign sign, const Bytes &magnitude) {
    return from_sign_and_magnitude(sign == Sign::Negative, magnitude);
  }

  static Integer from_magnitude_be(Sign sign, std::span<const Byte> magnitude) {
    return from_sign_and_magnitude(sign == Sign::Negative, magnitude);
  }

  static Integer from_decimal(std::string_view decimal) {
    if (decimal.empty()) {
      throw Value_Error("integer decimal spelling is empty",
                        Value_Error_Kind::Invalid_Integer);
    }
    bool negative = false;
    std::size_t offset = 0;
    if (decimal.front() == '-') {
      negative = true;
      offset = 1;
    } else if (decimal.front() == '+') {
      offset = 1;
    }
    if (offset == decimal.size()) {
      throw Value_Error("integer decimal spelling has no digits",
                        Value_Error_Kind::Invalid_Integer);
    }
    Integer result;
    for (std::size_t index = offset; index < decimal.size(); ++index) {
      const auto digit = decimal[index];
      if (digit < '0' || digit > '9') {
        throw Value_Error("integer decimal spelling contains a non-digit",
                          Value_Error_Kind::Invalid_Integer);
      }
      multiply_add(result.magnitude_, 10, static_cast<Byte>(digit - '0'));
    }
    if (result.magnitude_.empty()) {
      negative = false;
    }
    result.negative_ = negative;
    return result;
  }

  bool is_negative() const noexcept { return negative_; }

  bool is_zero() const noexcept { return magnitude_.empty(); }

  Sign sign() const noexcept {
    return negative_ ? Sign::Negative : Sign::Positive;
  }

  const Bytes &magnitude_be() const noexcept { return magnitude_; }

  std::optional<std::uint64_t> as_u64() const noexcept {
    if (negative_ || magnitude_.size() > sizeof(std::uint64_t)) {
      return std::nullopt;
    }
    return as_u64_magnitude();
  }

  std::optional<std::int64_t> as_i64() const noexcept {
    if (magnitude_.size() > sizeof(std::uint64_t)) {
      return std::nullopt;
    }
    const auto magnitude = as_u64_magnitude();
    if (negative_) {
      if (magnitude > (std::uint64_t{1} << 63u)) {
        return std::nullopt;
      }
      if (magnitude == (std::uint64_t{1} << 63u)) {
        return std::numeric_limits<std::int64_t>::min();
      }
      return -static_cast<std::int64_t>(magnitude);
    }
    if (magnitude >
        static_cast<std::uint64_t>(std::numeric_limits<std::int64_t>::max())) {
      return std::nullopt;
    }
    return static_cast<std::int64_t>(magnitude);
  }

  Bytes negative_cbor_magnitude() const {
    if (!negative_ || magnitude_.empty()) {
      return magnitude_;
    }
    Bytes result = magnitude_;
    subtract_one(result);
    return result;
  }

  std::string to_decimal() const {
    if (magnitude_.empty()) {
      return "0";
    }
    Bytes work = magnitude_;
    std::string digits;
    while (!work.empty()) {
      unsigned remainder = 0;
      for (auto &byte : work) {
        const unsigned value = (remainder << 8u) | byte;
        byte = static_cast<Byte>(value / 10u);
        remainder = value % 10u;
      }
      digits.push_back(static_cast<char>('0' + remainder));
      const auto first = first_nonzero(work);
      work.erase(work.begin(),
                 work.begin() + static_cast<std::ptrdiff_t>(first));
    }
    if (negative_) {
      digits.push_back('-');
    }
    std::reverse(digits.begin(), digits.end());
    return digits;
  }

  friend bool operator==(const Integer &left, const Integer &right) noexcept {
    return left.negative_ == right.negative_ &&
           left.magnitude_ == right.magnitude_;
  }

  friend bool operator!=(const Integer &left, const Integer &right) noexcept {
    return !(left == right);
  }

private:
  std::uint64_t as_u64_magnitude() const noexcept {
    std::uint64_t value = 0;
    for (const auto byte : magnitude_) {
      value = (value << 8u) | byte;
    }
    return value;
  }

  static std::size_t first_nonzero(std::span<const Byte> bytes) noexcept {
    std::size_t index = 0;
    while (index < bytes.size() && bytes[index] == 0) {
      ++index;
    }
    return index;
  }

  static void append_u64(Bytes &output, std::uint64_t value) {
    if (value == 0) {
      return;
    }
    std::array<Byte, sizeof(value)> bytes{};
    for (std::size_t index = bytes.size(); index != 0; --index) {
      bytes[index - 1] = static_cast<Byte>(value & 0xffu);
      value >>= 8u;
    }
    const auto first = first_nonzero(bytes);
    output.assign(bytes.begin() + static_cast<std::ptrdiff_t>(first),
                  bytes.end());
  }

  static void multiply_add(Bytes &value, unsigned multiplier, unsigned addend) {
    unsigned carry = addend;
    for (auto index = value.rbegin(); index != value.rend(); ++index) {
      const unsigned next = static_cast<unsigned>(*index) * multiplier + carry;
      *index = static_cast<Byte>(next & 0xffu);
      carry = next >> 8u;
    }
    while (carry != 0) {
      value.insert(value.begin(), static_cast<Byte>(carry & 0xffu));
      carry >>= 8u;
    }
  }

  static void subtract_one(Bytes &value) {
    for (auto index = value.rbegin(); index != value.rend(); ++index) {
      if (*index != 0) {
        --*index;
        break;
      }
      *index = 0xffu;
    }
    const auto first = first_nonzero(value);
    value.erase(value.begin(),
                value.begin() + static_cast<std::ptrdiff_t>(first));
  }

  bool negative_ = false;
  Bytes magnitude_;
};

struct Float_Value final {
  std::uint8_t width = 64;
  std::uint64_t raw_bits = 0;

  static Float_Value float16(std::uint16_t bits) noexcept { return {16, bits}; }

  static Float_Value float32(std::uint32_t bits) noexcept { return {32, bits}; }

  static Float_Value float64(std::uint64_t bits) noexcept { return {64, bits}; }

  void validate() const {
    if (width != 16 && width != 32 && width != 64) {
      throw Value_Error("float width must be 16, 32, or 64",
                        Value_Error_Kind::Unsupported_Type);
    }
    if (width == 16 && (raw_bits >> 16u) != 0) {
      throw Value_Error("Float16 raw bits exceed 16 bits",
                        Value_Error_Kind::Invalid_Encoding);
    }
    if (width == 32 && (raw_bits >> 32u) != 0) {
      throw Value_Error("Float32 raw bits exceed 32 bits",
                        Value_Error_Kind::Invalid_Encoding);
    }
  }

  friend bool operator==(const Float_Value &,
                         const Float_Value &) noexcept = default;
};

class Value;
using Value_Array = std::vector<Value>;
using Value_Map = std::vector<std::pair<Value, Value>>;

namespace detail {
void ensure_utf8(std::string_view value);
}

enum class Value_Kind {
  Undefined,
  Null,
  Boolean,
  Integer,
  Float16,
  Float32,
  Float64,
  Bytes,
  Text,
  Array,
  Map,
};

/// Lossless tagged value model used by the maintained C++ facade.
class Value {
public:
  using Array = Value_Array;
  using Map = Value_Map;

  Value() : storage_(Undefined_Value{}) {}
  Value(Undefined_Value) : storage_(Undefined_Value{}) {}
  Value(Null_Value) : storage_(Null_Value{}) {}
  explicit Value(bool value) : storage_(value) {}
  explicit Value(::openkache::Integer value) : storage_(std::move(value)) {}
  explicit Value(Float_Value value) : storage_(value) { value.validate(); }
  explicit Value(Bytes value) : storage_(std::move(value)) {}
  explicit Value(Text value) : storage_(std::move(value)) {
    detail::ensure_utf8(std::get<Text>(storage_));
  }

  static Value undefined() { return Value(Undefined_Value{}); }

  static Value Undefined() { return undefined(); }

  static Value null() { return Value(Null_Value{}); }

  static Value Null() { return null(); }

  static Value boolean(bool value) { return Value(value); }

  static Value Boolean(bool value) { return boolean(value); }

  static Value integer(::openkache::Integer value) {
    return Value(std::move(value));
  }

  static Value integer(std::int64_t value) {
    return Value(::openkache::Integer(value));
  }

  template <typename T>
    requires(std::is_integral_v<T> &&
             !std::is_same_v<std::remove_cv_t<T>, std::int64_t>)
  static Value integer(T value) {
    if constexpr (std::is_unsigned_v<T>) {
      return Value(
          ::openkache::Integer::from_u64(static_cast<std::uint64_t>(value)));
    } else {
      return Value(::openkache::Integer(static_cast<std::int64_t>(value)));
    }
  }

  static Value integer(std::string_view decimal) {
    return Value(::openkache::Integer::from_decimal(decimal));
  }

  static Value Integer(std::int64_t value) { return integer(value); }

  template <typename T>
    requires(std::is_integral_v<T> &&
             !std::is_same_v<std::remove_cv_t<T>, std::int64_t>)
  static Value Integer(T value) {
    return integer(value);
  }

  static Value Integer(std::string_view decimal) { return integer(decimal); }

  static Value Integer(const openkache::Integer &value) {
    return integer(value);
  }

  static Value Integer_Value(std::string_view decimal) {
    return integer(decimal);
  }

  static Value float16(std::uint16_t bits) {
    return Value(Float_Value::float16(bits));
  }

  static Value Float16(std::uint16_t bits) { return float16(bits); }

  static Value float32(std::uint32_t bits) {
    return Value(Float_Value::float32(bits));
  }

  static Value Float32(std::uint32_t bits) { return float32(bits); }

  static Value float64(std::uint64_t bits) {
    return Value(Float_Value::float64(bits));
  }

  static Value Float64(std::uint64_t bits) { return float64(bits); }

  static Value bytes(Bytes value) { return Value(std::move(value)); }

  static Value ByteString(Bytes value) { return bytes(std::move(value)); }

  static Value text(std::string_view value) { return Value(Text(value)); }

  static Value TextString(Text value) { return text(std::move(value)); }

  static Value array(Array values) {
    try {
      auto result = Value(undefined());
      result.storage_ = std::make_shared<Array>(std::move(values));
      return result;
    } catch (const std::bad_alloc &) {
      throw Value_Error("structured array allocation failed",
                        Value_Error_Kind::Allocation);
    }
  }

  static Value Array_Value(Array values) { return array(std::move(values)); }

  static Value map(Map entries) {
    try {
      validate_map(entries);
      auto result = Value(undefined());
      result.storage_ = std::make_shared<Map>(std::move(entries));
      return result;
    } catch (const std::bad_alloc &) {
      throw Value_Error("structured map allocation failed",
                        Value_Error_Kind::Allocation);
    }
  }

  static Value Map_Value(Map entries) { return map(std::move(entries)); }

  Value(const Value &other) : storage_(clone_storage(other.storage_)) {}

  Value &operator=(const Value &other) {
    if (this != &other) {
      storage_ = clone_storage(other.storage_);
    }
    return *this;
  }

  Value(Value &&) noexcept = default;
  Value &operator=(Value &&) noexcept = default;
  ~Value() = default;

  Value_Kind kind() const noexcept {
    switch (storage_.index()) {
    case 0:
      return Value_Kind::Undefined;
    case 1:
      return Value_Kind::Null;
    case 2:
      return Value_Kind::Boolean;
    case 3: {
      return Value_Kind::Integer;
    }
    case 4: {
      const auto width = std::get<Float_Value>(storage_).width;
      return width == 16   ? Value_Kind::Float16
             : width == 32 ? Value_Kind::Float32
                           : Value_Kind::Float64;
    }
    case 5:
      return Value_Kind::Bytes;
    case 6:
      return Value_Kind::Text;
    case 7:
      return Value_Kind::Array;
    case 8:
      return Value_Kind::Map;
    default:
      return Value_Kind::Undefined;
    }
  }

  bool is_undefined() const noexcept { return kind() == Value_Kind::Undefined; }
  bool is_null() const noexcept { return kind() == Value_Kind::Null; }
  bool is_boolean() const noexcept { return kind() == Value_Kind::Boolean; }
  bool is_integer() const noexcept { return kind() == Value_Kind::Integer; }
  bool is_float() const noexcept {
    return kind() == Value_Kind::Float16 || kind() == Value_Kind::Float32 ||
           kind() == Value_Kind::Float64;
  }
  bool is_bytes() const noexcept { return kind() == Value_Kind::Bytes; }
  bool is_text() const noexcept { return kind() == Value_Kind::Text; }
  bool is_array() const noexcept { return kind() == Value_Kind::Array; }
  bool is_map() const noexcept { return kind() == Value_Kind::Map; }
  bool is_scalar_key() const noexcept { return !is_array() && !is_map(); }

  bool as_boolean() const {
    require_kind(Value_Kind::Boolean);
    return std::get<bool>(storage_);
  }

  const ::openkache::Integer &as_integer() const {
    require_kind(Value_Kind::Integer);
    return std::get<::openkache::Integer>(storage_);
  }

  Float_Value as_float() const {
    if (!is_float()) {
      throw Value_Error("value is not a float",
                        Value_Error_Kind::Unsupported_Type);
    }
    return std::get<Float_Value>(storage_);
  }

  std::uint8_t float_width() const { return as_float().width; }

  std::uint64_t float_raw_bits() const { return as_float().raw_bits; }

  const Bytes &as_bytes() const {
    require_kind(Value_Kind::Bytes);
    return std::get<Bytes>(storage_);
  }

  const Text &as_text() const {
    require_kind(Value_Kind::Text);
    return std::get<Text>(storage_);
  }

  const Array &as_array() const {
    require_kind(Value_Kind::Array);
    return *std::get<std::shared_ptr<Array>>(storage_);
  }

  const Map &as_map() const {
    require_kind(Value_Kind::Map);
    return *std::get<std::shared_ptr<Map>>(storage_);
  }

  std::size_t size() const {
    if (is_array())
      return as_array().size();
    if (is_map())
      return as_map().size();
    throw Value_Error("scalar value has no child size",
                      Value_Error_Kind::Unsupported_Type);
  }

  friend bool operator==(const Value &left, const Value &right) {
    return equal(left, right);
  }

  friend bool operator!=(const Value &left, const Value &right) {
    return !(left == right);
  }

  /// Returns a stable process-local hash suitable for scalar-key buckets.
  ///
  /// The result is only an indexing aid; callers must still compare values
  /// structurally to resolve hash collisions.
  static std::size_t scalar_hash_for_key(const Value &value) {
    if (!value.is_scalar_key()) {
      throw Value_Error("only scalar values can be hashed as map keys",
                        Value_Error_Kind::Non_Scalar_Key);
    }
    return scalar_hash(value);
  }

private:
  using Storage = std::variant<Undefined_Value, Null_Value, bool,
                               ::openkache::Integer, Float_Value, Bytes, Text,
                               std::shared_ptr<Array>, std::shared_ptr<Map>>;

  explicit Value(Storage storage) : storage_(std::move(storage)) {}

  void require_kind(Value_Kind expected) const {
    if (kind() != expected) {
      throw Value_Error("value has a different kind",
                        Value_Error_Kind::Unsupported_Type);
    }
  }

  static Storage clone_storage(const Storage &storage) {
    if (const auto *array = std::get_if<std::shared_ptr<Array>>(&storage)) {
      return std::make_shared<Array>(**array);
    }
    if (const auto *map = std::get_if<std::shared_ptr<Map>>(&storage)) {
      return std::make_shared<Map>(**map);
    }
    return storage;
  }

  static void validate_map(const Map &entries) {
    std::unordered_map<std::size_t, std::vector<std::size_t>> buckets;
    buckets.reserve(entries.size());
    for (std::size_t index = 0; index < entries.size(); ++index) {
      const auto &key = entries[index].first;
      if (!key.is_scalar_key()) {
        throw Value_Error("map key is not scalar",
                          Value_Error_Kind::Non_Scalar_Key);
      }
      auto &bucket = buckets[scalar_hash(key)];
      for (const auto previous : bucket) {
        if (key == entries[previous].first) {
          throw Value_Error("map contains a duplicate logical key",
                            Value_Error_Kind::Duplicate_Key);
        }
      }
      bucket.push_back(index);
    }
  }

  static std::size_t scalar_hash(const Value &value) {
    std::size_t hash = 0xcbf29ce484222325ull;
    const auto mix = [&hash](std::uint64_t part) {
      hash ^= static_cast<std::size_t>(part);
      hash *= static_cast<std::size_t>(0x100000001b3ull);
    };
    mix(static_cast<std::uint64_t>(value.kind()));
    switch (value.kind()) {
    case Value_Kind::Undefined:
    case Value_Kind::Null:
      break;
    case Value_Kind::Boolean:
      mix(value.as_boolean() ? 1u : 0u);
      break;
    case Value_Kind::Integer:
      mix(value.as_integer().is_negative() ? 1u : 0u);
      for (const auto byte : value.as_integer().magnitude_be()) {
        mix(byte);
      }
      break;
    case Value_Kind::Float16:
    case Value_Kind::Float32:
    case Value_Kind::Float64: {
      const auto floating = value.as_float();
      mix(floating.width);
      mix(floating.raw_bits);
      break;
    }
    case Value_Kind::Bytes:
      for (const auto byte : value.as_bytes()) {
        mix(byte);
      }
      break;
    case Value_Kind::Text:
      for (const auto byte :
           std::string_view(value.as_text().data(), value.as_text().size())) {
        mix(static_cast<Byte>(byte));
      }
      break;
    case Value_Kind::Array:
    case Value_Kind::Map:
      break;
    }
    return hash;
  }

  static bool equal(const Value &left, const Value &right) {
    if (left.kind() != right.kind()) {
      return false;
    }
    switch (left.kind()) {
    case Value_Kind::Undefined:
    case Value_Kind::Null:
      return true;
    case Value_Kind::Boolean:
      return left.as_boolean() == right.as_boolean();
    case Value_Kind::Integer:
      return left.as_integer() == right.as_integer();
    case Value_Kind::Float16:
    case Value_Kind::Float32:
    case Value_Kind::Float64:
      return left.as_float() == right.as_float();
    case Value_Kind::Bytes:
      return left.as_bytes() == right.as_bytes();
    case Value_Kind::Text:
      return left.as_text() == right.as_text();
    case Value_Kind::Array:
      return left.as_array() == right.as_array();
    case Value_Kind::Map: {
      const auto &lhs = left.as_map();
      const auto &rhs = right.as_map();
      if (lhs.size() != rhs.size())
        return false;
      std::vector<bool> matched(rhs.size(), false);
      for (const auto &[key, value] : lhs) {
        bool found = false;
        for (std::size_t index = 0; index < rhs.size(); ++index) {
          if (!matched[index] && key == rhs[index].first &&
              value == rhs[index].second) {
            matched[index] = true;
            found = true;
            break;
          }
        }
        if (!found)
          return false;
      }
      return true;
    }
    }
    return false;
  }

  Storage storage_;
};

namespace detail {

inline void ensure_utf8(std::string_view value) {
  const auto *bytes = reinterpret_cast<const Byte *>(value.data());
  std::size_t index = 0;
  while (index < value.size()) {
    const Byte first = bytes[index++];
    if (first <= 0x7f) {
      continue;
    }
    auto continuation = [&](std::size_t count) {
      if (index + count > value.size())
        return false;
      for (std::size_t offset = 0; offset < count; ++offset) {
        if ((bytes[index + offset] & 0xc0u) != 0x80u)
          return false;
      }
      index += count;
      return true;
    };
    if (first >= 0xc2 && first <= 0xdf) {
      if (!continuation(1)) {
        throw Value_Error("text is not valid UTF-8",
                          Value_Error_Kind::Invalid_Utf8);
      }
    } else if (first == 0xe0) {
      if (index >= value.size() || bytes[index] < 0xa0 || bytes[index] > 0xbf ||
          !continuation(2)) {
        throw Value_Error("text is not valid UTF-8",
                          Value_Error_Kind::Invalid_Utf8);
      }
    } else if (first >= 0xe1 && first <= 0xec) {
      if (!continuation(2)) {
        throw Value_Error("text is not valid UTF-8",
                          Value_Error_Kind::Invalid_Utf8);
      }
    } else if (first == 0xed) {
      if (index >= value.size() || bytes[index] < 0x80 || bytes[index] > 0x9f ||
          !continuation(2)) {
        throw Value_Error("text is not valid UTF-8",
                          Value_Error_Kind::Invalid_Utf8);
      }
    } else if (first >= 0xee && first <= 0xef) {
      if (!continuation(2)) {
        throw Value_Error("text is not valid UTF-8",
                          Value_Error_Kind::Invalid_Utf8);
      }
    } else if (first == 0xf0) {
      if (index >= value.size() || bytes[index] < 0x90 || bytes[index] > 0xbf ||
          !continuation(3)) {
        throw Value_Error("text is not valid UTF-8",
                          Value_Error_Kind::Invalid_Utf8);
      }
    } else if (first >= 0xf1 && first <= 0xf3) {
      if (!continuation(3)) {
        throw Value_Error("text is not valid UTF-8",
                          Value_Error_Kind::Invalid_Utf8);
      }
    } else if (first == 0xf4) {
      if (index >= value.size() || bytes[index] < 0x80 || bytes[index] > 0x8f ||
          !continuation(3)) {
        throw Value_Error("text is not valid UTF-8",
                          Value_Error_Kind::Invalid_Utf8);
      }
    } else {
      throw Value_Error("text is not valid UTF-8",
                        Value_Error_Kind::Invalid_Utf8);
    }
  }
}

inline void validate_limits(const Value_Limits &limits) {
  if (limits.max_bytes == 0) {
    throw Value_Error("structured value byte limit must be positive",
                      Value_Error_Kind::Resource_Limit, Value_Resource::Bytes,
                      1, 0);
  }
  if (limits.max_depth == 0) {
    throw Value_Error("structured value depth limit must be positive",
                      Value_Error_Kind::Resource_Limit, Value_Resource::Depth,
                      1, 0);
  }
  if (limits.max_depth > MAX_ALLOWED_VALUE_DEPTH) {
    throw Value_Error(
        "structured value depth limit exceeds the implementation maximum",
        Value_Error_Kind::Resource_Limit, Value_Resource::Depth,
        MAX_ALLOWED_VALUE_DEPTH, limits.max_depth);
  }
  if (limits.max_items == 0) {
    throw Value_Error("structured value item limit must be positive",
                      Value_Error_Kind::Resource_Limit, Value_Resource::Items,
                      1, 0);
  }
  if (limits.max_integer_bytes == 0) {
    throw Value_Error("structured value integer limit must be positive",
                      Value_Error_Kind::Resource_Limit,
                      Value_Resource::Integer_Bytes, 1, 0);
  }
}

inline void append_bytes(Bytes &output, std::span<const Byte> bytes,
                         const Value_Limits &limits) {
  const bool exceeds_limit =
      output.size() > limits.max_bytes ||
      bytes.size() >
          limits.max_bytes - std::min(limits.max_bytes, output.size());
  if (exceeds_limit) {
    const auto actual =
        bytes.size() > std::numeric_limits<std::size_t>::max() - output.size()
            ? std::numeric_limits<std::size_t>::max()
            : output.size() + bytes.size();
    throw Value_Error("structured value exceeds the byte limit",
                      Value_Error_Kind::Resource_Limit, Value_Resource::Bytes,
                      limits.max_bytes, actual);
  }
  output.insert(output.end(), bytes.begin(), bytes.end());
}

inline void append_bytes(Bytes &output, std::initializer_list<Byte> bytes,
                         const Value_Limits &limits) {
  append_bytes(output, std::span<const Byte>(bytes.begin(), bytes.size()),
               limits);
}

inline void append_head(Bytes &output, Byte major, std::uint64_t argument,
                        const Value_Limits &limits) {
  std::array<Byte, 9> encoded{};
  std::size_t length = 1;
  if (argument < 24) {
    encoded[0] =
        static_cast<Byte>((static_cast<std::uint64_t>(major) << 5u) | argument);
  } else if (argument <= 0xffu) {
    encoded[0] =
        static_cast<Byte>((static_cast<std::uint64_t>(major) << 5u) | 24u);
    encoded[1] = static_cast<Byte>(argument);
    length = 2;
  } else if (argument <= 0xffffu) {
    encoded[0] =
        static_cast<Byte>((static_cast<std::uint64_t>(major) << 5u) | 25u);
    const auto value = static_cast<std::uint16_t>(argument);
    encoded[1] = static_cast<Byte>(value >> 8u);
    encoded[2] = static_cast<Byte>(value);
    length = 3;
  } else if (argument <= 0xffffffffu) {
    encoded[0] =
        static_cast<Byte>((static_cast<std::uint64_t>(major) << 5u) | 26u);
    const auto value = static_cast<std::uint32_t>(argument);
    for (std::size_t index = 0; index != 4; ++index) {
      encoded[index + 1] = static_cast<Byte>(value >> (24u - 8u * index));
    }
    length = 5;
  } else {
    encoded[0] =
        static_cast<Byte>((static_cast<std::uint64_t>(major) << 5u) | 27u);
    for (std::size_t index = 0; index != 8; ++index) {
      encoded[index + 1] = static_cast<Byte>(argument >> (56u - 8u * index));
    }
    length = 9;
  }
  append_bytes(output, std::span<const Byte>(encoded.data(), length), limits);
}

inline std::uint64_t as_u64(std::span<const Byte> bytes) {
  if (bytes.size() > sizeof(std::uint64_t))
    return 0;
  std::uint64_t value = 0;
  for (Byte byte : bytes)
    value = (value << 8u) | byte;
  return value;
}

inline void encode_value(const Value &value, Bytes &output,
                         const Value_Limits &limits, std::size_t depth,
                         std::size_t &item_count);

inline void encode_integer(const Integer &integer, Bytes &output,
                           const Value_Limits &limits) {
  const auto &magnitude = integer.magnitude_be();
  if (magnitude.size() > limits.max_integer_bytes) {
    throw Value_Error("integer magnitude exceeds the configured limit",
                      Value_Error_Kind::Resource_Limit,
                      Value_Resource::Integer_Bytes, limits.max_integer_bytes,
                      magnitude.size());
  }
  Bytes cbor_magnitude = integer.negative_cbor_magnitude();
  if (cbor_magnitude.size() <= sizeof(std::uint64_t)) {
    const auto argument = as_u64(cbor_magnitude);
    detail::append_head(output, integer.is_negative() ? 1 : 0, argument,
                        limits);
    return;
  }
  detail::append_head(output, 6, integer.is_negative() ? 3 : 2, limits);
  detail::append_head(output, 2, cbor_magnitude.size(), limits);
  detail::append_bytes(output, cbor_magnitude, limits);
}

inline void encode_value(const Value &value, Bytes &output,
                         const Value_Limits &limits, std::size_t depth,
                         std::size_t &item_count) {
  if (++item_count > limits.max_items) {
    throw Value_Error("structured value item limit exceeded",
                      Value_Error_Kind::Resource_Limit, Value_Resource::Items,
                      limits.max_items, item_count);
  }
  switch (value.kind()) {
  case Value_Kind::Undefined:
    append_bytes(output, {0xf7}, limits);
    break;
  case Value_Kind::Null:
    append_bytes(output, {0xf6}, limits);
    break;
  case Value_Kind::Boolean:
    append_bytes(output, {static_cast<Byte>(value.as_boolean() ? 0xf5 : 0xf4)},
                 limits);
    break;
  case Value_Kind::Integer:
    encode_integer(value.as_integer(), output, limits);
    break;
  case Value_Kind::Float16: {
    const auto bits = static_cast<std::uint16_t>(value.float_raw_bits());
    append_bytes(output,
                 {0xf9, static_cast<Byte>(bits >> 8u), static_cast<Byte>(bits)},
                 limits);
    break;
  }
  case Value_Kind::Float32: {
    const auto bits = static_cast<std::uint32_t>(value.float_raw_bits());
    const std::array<Byte, 5> encoded{
        0xfa,
        static_cast<Byte>(bits >> 24u),
        static_cast<Byte>(bits >> 16u),
        static_cast<Byte>(bits >> 8u),
        static_cast<Byte>(bits),
    };
    append_bytes(output, encoded, limits);
    break;
  }
  case Value_Kind::Float64: {
    const auto bits = value.float_raw_bits();
    std::array<Byte, 9> encoded{0xfb};
    for (std::size_t index = 0; index != 8; ++index) {
      encoded[index + 1] = static_cast<Byte>(bits >> (56u - 8u * index));
    }
    append_bytes(output, encoded, limits);
    break;
  }
  case Value_Kind::Bytes:
    append_head(output, 2, value.as_bytes().size(), limits);
    append_bytes(output, value.as_bytes(), limits);
    break;
  case Value_Kind::Text:
    append_head(output, 3, value.as_text().size(), limits);
    append_bytes(output,
                 std::span<const Byte>(
                     reinterpret_cast<const Byte *>(value.as_text().data()),
                     value.as_text().size()),
                 limits);
    break;
  case Value_Kind::Array:
    if (depth >= limits.max_depth) {
      throw Value_Error("structured value depth limit exceeded",
                        Value_Error_Kind::Resource_Limit, Value_Resource::Depth,
                        limits.max_depth, depth + 1);
    }
    append_head(output, 4, value.as_array().size(), limits);
    for (const auto &child : value.as_array()) {
      encode_value(child, output, limits, depth + 1, item_count);
    }
    break;
  case Value_Kind::Map:
    if (depth >= limits.max_depth) {
      throw Value_Error("structured value depth limit exceeded",
                        Value_Error_Kind::Resource_Limit, Value_Resource::Depth,
                        limits.max_depth, depth + 1);
    }
    append_head(output, 5, value.as_map().size(), limits);
    for (const auto &[key, child] : value.as_map()) {
      encode_value(key, output, limits, depth + 1, item_count);
      encode_value(child, output, limits, depth + 1, item_count);
    }
    break;
  }
}

class Decoder {
public:
  Decoder(std::span<const Byte> bytes, Value_Limits limits)
      : bytes_(bytes), limits_(limits) {
    validate_limits(limits_);
    if (bytes_.size() > limits_.max_bytes) {
      throw Value_Error("structured value exceeds the byte limit",
                        Value_Error_Kind::Resource_Limit, Value_Resource::Bytes,
                        limits_.max_bytes, bytes_.size());
    }
  }

  Value decode() {
    if (bytes_.empty()) {
      throw Value_Error("structured value is truncated",
                        Value_Error_Kind::Truncated);
    }
    Value result = parse(0);
    if (cursor_ != bytes_.size()) {
      throw Value_Error("structured value contains trailing bytes",
                        Value_Error_Kind::Trailing_Bytes);
    }
    return result;
  }

private:
  struct Head {
    Byte major;
    std::uint64_t argument;
    Byte additional;
  };

  Byte read_byte() {
    if (cursor_ >= bytes_.size()) {
      throw Value_Error("structured value is truncated",
                        Value_Error_Kind::Truncated);
    }
    return bytes_[cursor_++];
  }

  std::uint64_t read_uint(std::size_t count) {
    if (count > bytes_.size() - cursor_) {
      throw Value_Error("structured value is truncated",
                        Value_Error_Kind::Truncated);
    }
    std::uint64_t value = 0;
    for (std::size_t index = 0; index != count; ++index) {
      value = (value << 8u) | bytes_[cursor_++];
    }
    return value;
  }

  Head read_head() {
    const auto initial = read_byte();
    const auto major = static_cast<Byte>(initial >> 5u);
    const auto additional = static_cast<Byte>(initial & 0x1fu);
    if (additional == 31) {
      throw Value_Error("indefinite-length CBOR is not supported",
                        Value_Error_Kind::Invalid_Encoding);
    }
    if (additional < 24)
      return {major, additional, additional};
    if (additional == 24)
      return {major, read_uint(1), additional};
    if (additional == 25)
      return {major, read_uint(2), additional};
    if (additional == 26)
      return {major, read_uint(4), additional};
    if (additional == 27)
      return {major, read_uint(8), additional};
    throw Value_Error("reserved CBOR additional information",
                      Value_Error_Kind::Invalid_Encoding);
  }

  Value parse(std::size_t depth) {
    if (++item_count_ > limits_.max_items) {
      throw Value_Error("structured value item limit exceeded",
                        Value_Error_Kind::Resource_Limit, Value_Resource::Items,
                        limits_.max_items, item_count_);
    }
    const auto head = read_head();
    switch (head.major) {
    case 0: {
      enforce_integer_limit(integer_magnitude_size(head.argument));
      Bytes magnitude;
      append_u64(magnitude, head.argument);
      return Value::integer(Integer::from_sign_and_magnitude(false, magnitude));
    }
    case 1:
      return decode_negative(head.argument);
    case 2:
      return Value::bytes(
          read_bytes(head.argument, Value_Resource::Bytes, limits_.max_bytes));
    case 3: {
      auto value =
          read_bytes(head.argument, Value_Resource::Bytes, limits_.max_bytes);
      Text text(value.begin(), value.end());
      detail::ensure_utf8(text);
      return Value::text(std::move(text));
    }
    case 4:
      return parse_array(head.argument, depth);
    case 5:
      return parse_map(head.argument, depth);
    case 6:
      return parse_tag(head.argument, depth);
    case 7:
      return parse_simple(head);
    default:
      throw Value_Error("unsupported CBOR major type",
                        Value_Error_Kind::Unsupported_Type);
    }
  }

  Value decode_negative(std::uint64_t argument) {
    // A major-type-1 argument represents `-1-n`.  The mathematical
    // magnitude is therefore `n+1`, which needs nine bytes when `n` is
    // UINT64_MAX (the previous implementation accidentally decoded that
    // boundary as -UINT64_MAX).
    if (argument == std::numeric_limits<std::uint64_t>::max()) {
      enforce_integer_limit(sizeof(argument) + 1);
      Bytes magnitude(sizeof(argument) + 1, 0);
      magnitude.front() = 1;
      return Value::integer(Integer::from_sign_and_magnitude(true, magnitude));
    }
    enforce_integer_limit(integer_magnitude_size(argument + 1));
    Bytes magnitude;
    append_u64(magnitude, argument + 1);
    return Value::integer(Integer::from_sign_and_magnitude(true, magnitude));
  }

  void enforce_integer_limit(std::size_t magnitude_size) const {
    if (magnitude_size > limits_.max_integer_bytes) {
      throw Value_Error("integer magnitude exceeds the configured limit",
                        Value_Error_Kind::Resource_Limit,
                        Value_Resource::Integer_Bytes,
                        limits_.max_integer_bytes, magnitude_size);
    }
  }

  static std::size_t integer_magnitude_size(std::uint64_t value) noexcept {
    if (value == 0) {
      return 0;
    }
    std::size_t bytes = 0;
    while (value != 0) {
      ++bytes;
      value >>= 8u;
    }
    return bytes;
  }

  Value parse_tag(std::uint64_t tag, std::size_t depth) {
    if (tag != 2 && tag != 3) {
      throw Value_Error("unsupported CBOR tag",
                        Value_Error_Kind::Unsupported_Type);
    }
    const auto head = read_head();
    if (head.major != 2) {
      throw Value_Error("CBOR bignum tag must wrap a byte string",
                        Value_Error_Kind::Invalid_Integer);
    }
    auto magnitude = read_bytes(head.argument, Value_Resource::Integer_Bytes,
                                limits_.max_integer_bytes);
    if (magnitude.empty() || magnitude.front() == 0) {
      throw Value_Error("CBOR bignum magnitude is not minimal",
                        Value_Error_Kind::Invalid_Integer);
    }
    if (magnitude.size() > limits_.max_integer_bytes) {
      throw Value_Error("integer magnitude exceeds the configured limit",
                        Value_Error_Kind::Resource_Limit,
                        Value_Resource::Integer_Bytes,
                        limits_.max_integer_bytes, magnitude.size());
    }
    if (tag == 3) {
      if (magnitude.size() == limits_.max_integer_bytes &&
          std::all_of(magnitude.begin(), magnitude.end(),
                      [](Byte byte) { return byte == 0xffu; })) {
        throw Value_Error("integer magnitude exceeds the configured limit",
                          Value_Error_Kind::Resource_Limit,
                          Value_Resource::Integer_Bytes,
                          limits_.max_integer_bytes, magnitude.size() + 1);
      }
      increment_be(magnitude);
    }
    (void)depth;
    return Value::integer(
        Integer::from_sign_and_magnitude(tag == 3, magnitude));
  }

  Value parse_simple(const Head &head) {
    if (head.additional == 20)
      return Value::boolean(false);
    if (head.additional == 21)
      return Value::boolean(true);
    if (head.additional == 22)
      return Value::null();
    if (head.additional == 23)
      return Value::undefined();
    if (head.additional == 25) {
      return Value::float16(static_cast<std::uint16_t>(head.argument));
    }
    if (head.additional == 26) {
      return Value::float32(static_cast<std::uint32_t>(head.argument));
    }
    if (head.additional == 27)
      return Value::float64(head.argument);
    throw Value_Error("unsupported CBOR simple value",
                      Value_Error_Kind::Unsupported_Type);
  }

  Value parse_array(std::uint64_t count, std::size_t depth) {
    if (depth >= limits_.max_depth) {
      throw Value_Error("structured value depth limit exceeded",
                        Value_Error_Kind::Resource_Limit, Value_Resource::Depth,
                        limits_.max_depth, depth + 1);
    }
    if (count > static_cast<std::uint64_t>(limits_.max_items)) {
      throw Value_Error("structured value item limit exceeded",
                        Value_Error_Kind::Resource_Limit, Value_Resource::Items,
                        limits_.max_items,
                        count > std::numeric_limits<std::size_t>::max()
                            ? std::numeric_limits<std::size_t>::max()
                            : static_cast<std::size_t>(count));
    }
    const auto child_count = static_cast<std::size_t>(count);
    if (child_count > limits_.max_items - item_count_) {
      throw Value_Error("structured value item limit exceeded",
                        Value_Error_Kind::Resource_Limit, Value_Resource::Items,
                        limits_.max_items, item_count_ + child_count);
    }
    Value::Array values;
    values.reserve(child_count);
    for (std::size_t index = 0; index < child_count; ++index) {
      values.push_back(parse(depth + 1));
    }
    return Value::array(std::move(values));
  }

  Value parse_map(std::uint64_t count, std::size_t depth) {
    if (depth >= limits_.max_depth) {
      throw Value_Error("structured value depth limit exceeded",
                        Value_Error_Kind::Resource_Limit, Value_Resource::Depth,
                        limits_.max_depth, depth + 1);
    }
    if (count > static_cast<std::uint64_t>(limits_.max_items)) {
      throw Value_Error("structured value item limit exceeded",
                        Value_Error_Kind::Resource_Limit, Value_Resource::Items,
                        limits_.max_items,
                        count > std::numeric_limits<std::size_t>::max()
                            ? std::numeric_limits<std::size_t>::max()
                            : static_cast<std::size_t>(count));
    }
    const auto entry_count = static_cast<std::size_t>(count);
    if (entry_count > (std::numeric_limits<std::size_t>::max() / 2u)) {
      throw Value_Error("structured value item limit exceeded",
                        Value_Error_Kind::Resource_Limit, Value_Resource::Items,
                        limits_.max_items,
                        std::numeric_limits<std::size_t>::max());
    }
    const auto child_count = entry_count * 2u;
    if (child_count > limits_.max_items - item_count_) {
      throw Value_Error("structured value item limit exceeded",
                        Value_Error_Kind::Resource_Limit, Value_Resource::Items,
                        limits_.max_items, item_count_ + child_count);
    }
    Value::Map entries;
    entries.reserve(entry_count);
    std::unordered_map<std::size_t, std::vector<std::size_t>> buckets;
    buckets.reserve(entry_count);
    for (std::size_t index = 0; index < entry_count; ++index) {
      Value key = parse(depth + 1);
      if (!key.is_scalar_key()) {
        throw Value_Error("map key is not scalar",
                          Value_Error_Kind::Non_Scalar_Key);
      }
      auto &bucket = buckets[Value::scalar_hash_for_key(key)];
      for (const auto previous : bucket) {
        if (entries[previous].first == key) {
          throw Value_Error("map contains a duplicate logical key",
                            Value_Error_Kind::Duplicate_Key);
        }
      }
      bucket.push_back(entries.size());
      entries.emplace_back(std::move(key), parse(depth + 1));
    }
    return Value::map(std::move(entries));
  }

  Bytes read_bytes(std::uint64_t count, Value_Resource resource,
                   std::size_t limit) {
    if (count > limit) {
      throw Value_Error("structured value bytes exceed the configured limit",
                        Value_Error_Kind::Resource_Limit, resource, limit,
                        count > std::numeric_limits<std::size_t>::max()
                            ? std::numeric_limits<std::size_t>::max()
                            : static_cast<std::size_t>(count));
    }
    if (count > bytes_.size() - cursor_) {
      throw Value_Error("structured value is truncated",
                        Value_Error_Kind::Truncated);
    }
    const auto begin = bytes_.begin() + static_cast<std::ptrdiff_t>(cursor_);
    Bytes result(begin, begin + static_cast<std::ptrdiff_t>(count));
    cursor_ += static_cast<std::size_t>(count);
    return result;
  }

  static void append_u64(Bytes &output, std::uint64_t value) {
    if (value == 0)
      return;
    std::array<Byte, sizeof(value)> bytes{};
    for (std::size_t index = bytes.size(); index != 0; --index) {
      bytes[index - 1] = static_cast<Byte>(value & 0xffu);
      value >>= 8u;
    }
    const auto first = std::find_if(bytes.begin(), bytes.end(),
                                    [](Byte byte) { return byte != 0; });
    output.assign(first, bytes.end());
  }

  static void increment_be(Bytes &value) {
    for (auto index = value.rbegin(); index != value.rend(); ++index) {
      if (*index != 0xffu) {
        ++*index;
        return;
      }
      *index = 0;
    }
    value.insert(value.begin(), 1);
  }

  std::span<const Byte> bytes_;
  Value_Limits limits_;
  std::size_t cursor_ = 0;
  std::size_t item_count_ = 0;
};

} // namespace detail

/// Encodes one complete StructuredValue-CBOR-v1 item.
inline Bytes encode_structured_value(const Value &value,
                                     Value_Limits limits = {}) {
  detail::validate_limits(limits);
  try {
    Bytes output;
    output.reserve(std::min<std::size_t>(limits.max_bytes, 256));
    std::size_t item_count = 0;
    detail::encode_value(value, output, limits, 0, item_count);
    return output;
  } catch (const std::bad_alloc &) {
    throw Value_Error("structured value allocation failed",
                      Value_Error_Kind::Allocation);
  }
}

/// Decodes exactly one complete StructuredValue-CBOR-v1 item.
inline Value decode_structured_value(std::span<const Byte> bytes,
                                     Value_Limits limits = {}) {
  try {
    return detail::Decoder(bytes, limits).decode();
  } catch (const std::bad_alloc &) {
    throw Value_Error("structured value allocation failed",
                      Value_Error_Kind::Allocation);
  }
}

inline Value decode_structured_value(const Bytes &bytes,
                                     Value_Limits limits = {}) {
  return decode_structured_value(std::span<const Byte>(bytes), limits);
}

namespace structured_value_cbor_v1 {
using ::openkache::decode_structured_value;
using ::openkache::encode_structured_value;
using ::openkache::Value;
using ::openkache::Value_Error;
using ::openkache::Value_Error_Kind;
using ::openkache::Value_Limits;
} // namespace structured_value_cbor_v1

} // namespace openkache

#endif /* OPENKACHE_VALUE_HPP */
