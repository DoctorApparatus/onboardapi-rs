/* C ABI between the Rust crate and the C++ shim. Keep in sync with src/lib.rs. */
#ifndef OBAPI_SHIM_H
#define OBAPI_SHIM_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

enum obapi_status {
    OBAPI_OK = 0,
    OBAPI_ERR_EXCEPTION = -1, /* std::exception thrown by onboardapi / ddkit */
    OBAPI_ERR_UNKNOWN = -2,   /* non-std exception */
    OBAPI_ERR_WIRE = -3,      /* malformed payload from Rust */
    OBAPI_ERR_INVALID = -4,   /* null or invalid handle */
};

/* Called from onboardapi threads for every incoming operation. */
typedef void (*obapi_dispatch_fn)(void* ctx, uint32_t op, const uint8_t* data, size_t len);

typedef struct obapi_vtable {
    const char* module; /* e.g. "M_CoordinateFrame" */
    const char* role;   /* "Service" or "Client" */

    /* dispatch == NULL creates a write-only endpoint. */
    int32_t (*create)(int32_t domain, const char* name, size_t name_len, void* ctx, obapi_dispatch_fn dispatch,
                      void** out);
    int32_t (*send)(void* handle, uint32_t op, const uint8_t* data, size_t len);
    /* Stops callbacks, then destroys the endpoint. No callback runs after this returns. */
    void (*destroy)(void* handle);
    bool (*is_valid)(void* handle);

    /* Client only, NULL for services. */
    int32_t (*wait_until_connected)(void* handle);
    int32_t (*set_name)(void* handle, const char* name, size_t name_len);
} obapi_vtable;

/* Copies the calling thread's last error message into buf (truncated), returns its full length. */
size_t obapi_last_error(char* buf, size_t cap);

#ifdef __cplusplus
}
#endif

#endif
