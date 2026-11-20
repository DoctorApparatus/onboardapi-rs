#include "obapi_runtime.hpp"

#include <algorithm>

namespace {
thread_local std::string g_last_error;
}

namespace obapi {
void set_last_error(std::string msg) { g_last_error = std::move(msg); }
}  // namespace obapi

extern "C" size_t obapi_last_error(char* buf, size_t cap) {
    if (buf && cap) {
        size_t n = std::min(cap, g_last_error.size());
        std::copy_n(g_last_error.data(), n, buf);
    }
    return g_last_error.size();
}
