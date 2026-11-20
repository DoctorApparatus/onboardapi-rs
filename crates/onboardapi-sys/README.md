# onboardapi-sys

Raw FFI for the [`onboardapi`](https://crates.io/crates/onboardapi) crate. A generated C++ shim exposes each onboardapi interface module as a C vtable, and all data crosses the boundary in one small binary format.

Use `onboardapi` instead.

Building needs the onboardapi C++ SDK (EULA-RME-SDK-1.0, not bundled) and a C++17 compiler:

```sh
export ONBOARDAPI_SDK_DIR=/path/to/sdk   # contains include/ and lib/
```

The shim `static_assert`s every struct field, typedef and enum ordinal against the SDK headers, so an SDK that doesn't match the data model fails the build instead of misreading data.

> Unofficial community bindings, not affiliated with or endorsed by Rheinmetall.
