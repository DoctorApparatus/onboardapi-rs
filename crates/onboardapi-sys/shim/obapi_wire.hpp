// Binary wire format shared with `onboardapi::wire` (Rust):
//   bool          u8 (0/1)
//   integers      little endian, fixed width
//   float/double  IEEE-754 little endian
//   string        u32 byte length + UTF-8 bytes
//   sequence<T>   u32 element count + elements (optional types are sequences of <= 1)
//   enum          u32 ordinal
//   struct        fields in declaration order
#pragma once

#include <cstdint>
#include <cstring>
#include <stdexcept>
#include <string>
#include <type_traits>
#include <vector>

namespace obapi {

struct WireError : std::runtime_error {
    using std::runtime_error::runtime_error;
};

class Writer {
public:
    void bytes(void const* p, size_t n) {
        auto const* b = static_cast<uint8_t const*>(p);
        m_buf.insert(m_buf.end(), b, b + n);
    }
    template <typename T>
    void raw(T v) {
        static_assert(std::is_arithmetic<T>::value, "raw() takes arithmetic types");
        // All supported targets are little endian.
        bytes(&v, sizeof v);
    }
    void len(size_t n) {
        if (n > UINT32_MAX) throw WireError("sequence too long for the wire format");
        raw(static_cast<uint32_t>(n));
    }
    uint8_t const* data() const { return m_buf.data(); }
    size_t size() const { return m_buf.size(); }

private:
    std::vector<uint8_t> m_buf;
};

class Reader {
public:
    Reader(uint8_t const* data, size_t len) : m_data(data), m_len(len) {}

    template <typename T>
    T raw() {
        static_assert(std::is_arithmetic<T>::value, "raw() takes arithmetic types");
        T v;
        std::memcpy(&v, take(sizeof v), sizeof v);
        return v;
    }
    uint8_t const* take(size_t n) {
        if (m_len - m_pos < n) throw WireError("payload truncated");
        auto const* p = m_data + m_pos;
        m_pos += n;
        return p;
    }
    size_t len() {
        auto n = raw<uint32_t>();
        // Elements of empty structs take zero bytes, so a count above the remaining
        // byte count is legal; only reject counts that cannot be sane.
        if (n > m_len - m_pos && n > (1u << 20)) throw WireError("sequence length exceeds payload");
        return n;
    }
    void finish() const {
        if (m_pos != m_len) throw WireError("trailing bytes in payload");
    }

private:
    uint8_t const* m_data;
    size_t m_len;
    size_t m_pos = 0;
};

// Entry points used by generated code: `obapi::put(w, x)` / `obapi::get(r, x)`.
// They dispatch to the `obapi_put` / `obapi_get` overloads below or, for generated
// structs, to the overloads in the struct's namespace (found by ADL).
template <typename T>
void put(Writer& w, T const& v);
template <typename T>
void get(Reader& r, T& v);

inline void obapi_put(Writer& w, bool v) { w.raw<uint8_t>(v ? 1 : 0); }
inline void obapi_get(Reader& r, bool& v) {
    auto b = r.raw<uint8_t>();
    if (b > 1) throw WireError("invalid bool");
    v = b == 1;
}

#define OBAPI_ARITH(T)                                         \
    inline void obapi_put(Writer& w, T v) { w.raw<T>(v); }     \
    inline void obapi_get(Reader& r, T& v) { v = r.raw<T>(); }
OBAPI_ARITH(char)
OBAPI_ARITH(int8_t)
OBAPI_ARITH(uint8_t)
OBAPI_ARITH(int16_t)
OBAPI_ARITH(uint16_t)
OBAPI_ARITH(int32_t)
OBAPI_ARITH(uint32_t)
OBAPI_ARITH(int64_t)
OBAPI_ARITH(uint64_t)
OBAPI_ARITH(float)
OBAPI_ARITH(double)
#undef OBAPI_ARITH

inline void obapi_put(Writer& w, std::string const& v) {
    w.len(v.size());
    w.bytes(v.data(), v.size());
}
inline void obapi_get(Reader& r, std::string& v) {
    auto n = r.len();
    auto const* p = r.take(n);
    v.assign(reinterpret_cast<char const*>(p), n);
}

template <typename E, typename std::enable_if<std::is_enum<E>::value, int>::type = 0>
void obapi_put(Writer& w, E v) {
    w.raw<uint32_t>(static_cast<uint32_t>(v));
}
template <typename E, typename std::enable_if<std::is_enum<E>::value, int>::type = 0>
void obapi_get(Reader& r, E& v) {
    // Range is checked on the Rust side when decoding; values from Rust are always valid.
    v = static_cast<E>(r.raw<uint32_t>());
}

inline void obapi_put(Writer& w, std::vector<bool> const& v) {
    w.len(v.size());
    for (bool b : v) obapi_put(w, b);
}
inline void obapi_get(Reader& r, std::vector<bool>& v) {
    auto n = r.len();
    v.assign(n, false);
    for (size_t i = 0; i < n; ++i) {
        bool b;
        obapi_get(r, b);
        v[i] = b;
    }
}

template <typename T, typename A>
void obapi_put(Writer& w, std::vector<T, A> const& v) {
    w.len(v.size());
    for (auto const& x : v) put(w, x);
}
template <typename T, typename A>
void obapi_get(Reader& r, std::vector<T, A>& v) {
    auto n = r.len();
    v.clear();
    v.resize(n);
    for (auto& x : v) get(r, x);
}

template <typename T>
void put(Writer& w, T const& v) {
    obapi_put(w, v);
}
template <typename T>
void get(Reader& r, T& v) {
    obapi_get(r, v);
}

}  // namespace obapi
