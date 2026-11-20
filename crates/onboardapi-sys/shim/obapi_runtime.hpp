// Generic glue between the generated per-module code and the C ABI in obapi.h.
#pragma once

#include "obapi.h"
#include "obapi_wire.hpp"

#include <atomic>
#include <memory>
#include <mutex>
#include <shared_mutex>
#include <string>
#include <utility>

namespace obapi {

struct UnknownOp : WireError {
    explicit UnknownOp(uint32_t op) : WireError("unknown operation id " + std::to_string(op)) {}
};

void set_last_error(std::string msg);

/// Runs `f`, translating C++ exceptions into status codes + thread-local message.
template <typename F>
int32_t guard(F&& f) noexcept {
    try {
        f();
        return OBAPI_OK;
    } catch (WireError const& e) {
        set_last_error(e.what());
        return OBAPI_ERR_WIRE;
    } catch (std::exception const& e) {
        set_last_error(e.what());
        return OBAPI_ERR_EXCEPTION;
    } catch (...) {
        set_last_error("unknown C++ exception");
        return OBAPI_ERR_UNKNOWN;
    }
}

/// Forwards encoded operations to Rust. `close()` blocks until running callbacks finish
/// and prevents new ones, so the Rust context can be freed afterwards.
class Dispatcher {
public:
    void open(void* ctx, obapi_dispatch_fn fn) {
        std::unique_lock<std::shared_mutex> lk(m_mx);
        m_ctx = ctx;
        m_fn = fn;
    }
    void close() {
        std::unique_lock<std::shared_mutex> lk(m_mx);
        m_fn = nullptr;
    }
    void dispatch(uint32_t op, Writer const& w) {
        std::shared_lock<std::shared_mutex> lk(m_mx);
        if (m_fn) m_fn(m_ctx, op, w.data(), w.size());
    }

private:
    std::shared_mutex m_mx;
    void* m_ctx = nullptr;
    obapi_dispatch_fn m_fn = nullptr;
};

/// Owns the callbacks object and the onboardapi endpoint. Member order matters: the
/// endpoint is destroyed before the callbacks it references.
template <typename Ep, typename Cb>
struct Holder {
    Cb callbacks;
    Ep endpoint;
    std::mutex send_mx;
};

template <typename Ep, typename Cb, void (*Send)(Holder<Ep, Cb>&, uint32_t, Reader&), bool IsClient>
struct Endpoint {
    using H = Holder<Ep, Cb>;

    static int32_t create(int32_t domain, char const* name, size_t name_len, void* ctx, obapi_dispatch_fn fn,
                          void** out) noexcept {
        if (!out || (!name && name_len)) {
            set_last_error("null argument");
            return OBAPI_ERR_INVALID;
        }
        return guard([&] {
            auto h = std::make_unique<H>();
            std::string service(name ? name : "", name_len);
            if (fn) {
                h->callbacks.dispatcher.open(ctx, fn);
                h->endpoint = Ep::create(static_cast<int>(domain), service, h->callbacks);
            } else {
                h->endpoint = Ep::createWriteOnly(static_cast<int>(domain), service);
            }
            *out = h.release();
        });
    }

    static int32_t send(void* p, uint32_t op, uint8_t const* data, size_t len) noexcept {
        if (!p || (!data && len)) {
            set_last_error("null argument");
            return OBAPI_ERR_INVALID;
        }
        return guard([&] {
            auto& h = *static_cast<H*>(p);
            Reader r(data, len);
            std::lock_guard<std::mutex> lk(h.send_mx);
            Send(h, op, r);
        });
    }

    static void destroy(void* p) noexcept {
        if (!p) return;
        auto* h = static_cast<H*>(p);
        h->callbacks.dispatcher.close();
        try {
            delete h;
        } catch (...) {
        }
    }

    static bool is_valid(void* p) noexcept { return p && static_cast<H*>(p)->endpoint.isValid(); }

    static int32_t wait_until_connected(void* p) noexcept {
        if (!p) return OBAPI_ERR_INVALID;
        return guard([&] { waitImpl(&static_cast<H*>(p)->endpoint, 0); });
    }

    static int32_t set_name(void* p, char const* name, size_t len) noexcept {
        if (!p || (!name && len)) return OBAPI_ERR_INVALID;
        return guard([&] {
            auto& h = *static_cast<H*>(p);
            std::lock_guard<std::mutex> lk(h.send_mx);
            setNameImpl(&h.endpoint, std::string(name ? name : "", len), 0);
        });
    }

    static constexpr obapi_vtable vtable(char const* module, char const* role) {
        return obapi_vtable{
            module, role, &create, &send, &destroy, &is_valid,
            IsClient ? &wait_until_connected : nullptr,
            IsClient ? &set_name : nullptr,
        };
    }

private:
    // Overloads pick the member when it exists (Client) and a no-op otherwise (Service).
    template <typename E>
    static auto waitImpl(E* e, int) -> decltype(e->waitUntilConnected()) {
        e->waitUntilConnected();
    }
    static void waitImpl(void*, long) {}

    template <typename E>
    static auto setNameImpl(E* e, std::string n, int) -> decltype(e->setName(n)) {
        e->setName(std::move(n));
    }
    static void setNameImpl(void*, std::string, long) {}
};

}  // namespace obapi
