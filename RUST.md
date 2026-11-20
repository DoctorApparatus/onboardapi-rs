# onboardapi for Rust

Safe, async Rust bindings for onboardapi 9.10.0. All 52 interface modules are covered, with 716 types and 485 operations. The bindings are generated from `datamodel/*.rmodel` and run on top of the official C++ SDK.

## Quick start

```sh
just fetch-sdk        # ~400 MB download, unpacks the x86_64 Release build into .sdk/
just test             # sends every operation of every module through DDS once
just gnss service     # pane 1: simulated GNSS publishing at 10 Hz
just gnss client      # pane 2: prints what it receives
```

```toml
# Cargo.toml of your simulation
[dependencies]
onboardapi = { path = "../onboardapi/crates/onboardapi" }
tokio = { version = "1", features = ["full"] }
```

## Why not bindgen

The C++ API is generated code with
* virtual callback interfaces (`IService`/`IClient`) that you subclass,
* `std::string`/`std::vector` structs,
* SFINAE-templated factories (`Service::create<T, Derived>`).

bindgen can't subclass C++ classes or instantiate those templates. It would hand you
raw `std::` layouts. Instead, `onboardapi-codegen` reads the `.rmodel` IDL and writes both sides of a small `extern "C"` boundary:

```
your code ──► onboardapi (safe, async)      generated Rust types, messages, handlers, typed senders
                 │  binary wire format (LE, u32 lengths) — shim/obapi_wire.hpp ⇄ src/wire.rs
              onboardapi-sys ──► C++ shim   generated: subclasses IService/IClient, one vtable per role
                 │
              libonboardapi-cyclone.so (official SDK) ──► Cyclone DDS
```

The shim has `static_assert`s on every struct field, typedef and enum ordinal against
the SDK headers. If a future SDK drifts from the `.rmodel` files, the build fails and
data is never misread. The C++ signatures are checked the same way, through `override`.

## Crates

| crate | what |
|---|---|
| `crates/onboardapi` | the API you use |
| `crates/onboardapi-sys` | FFI + `build.rs` that compiles the shim (`cc`) and links the SDK |
| `crates/onboardapi-codegen` | `.rmodel` parser + Rust/C++ emitters (`just codegen`) |

Generated files (commit them, regenerate with `just codegen`):
`crates/onboardapi/src/generated/`, `crates/onboardapi/tests/all_operations.rs`,
`crates/onboardapi-sys/shim/generated/`, `crates/onboardapi-sys/src/generated.rs`.

## Usage

Each IDL module `M_Xyz` is the Rust module `onboardapi::xyz`:

| C++ | Rust |
|---|---|
| `M_Common::PositionType` | `common::PositionType` (serde names stay `PascalCase` as in the IDL) |
| `M_Common::StorageKind::Type` | `common::StorageKind` (strum: `Display`, `FromStr`, `EnumIter`, …) |
| `sequence<T, 1>` (`…OptionalType`) | `Option<T>` |
| `sequence<T>` | `Vec<T>` |
| `M_X::IService` operations | `x::ServiceMessage` variants / `x::ServiceHandler` methods |
| `M_X::IClient` operations | `x::ClientMessage` variants / `x::ClientHandler` methods |
| `M_X::Service::create(domain, name, cb)` | `x::Service::create(domain, name)? -> (Service, Inbox<ServiceMessage>)` |
| `service->ReportPositions(...)` | `service.report_positions(key, &list, false)?` |
| `client.waitUntilConnected()` | `client.connected().await?` |

### Service

```rust
use onboardapi::coordinate_frame::{CoordinateFrameStatusType, Service, ServiceMessage};

let (service, mut inbox) = Service::create(0, "gnss_sim")?;
service.report_coordinate_frame_status(&CoordinateFrameStatusType::default(), false)?;

while let Some(msg) = inbox.recv().await {
    match msg {
        ServiceMessage::NotifyPosition { key_id, position } => { /* move the vehicle */ }
        _ => {}
    }
}
// Dropping `service` sends IsRemoved=true for all late-join data.
```

### Client with a handler

```rust
use onboardapi::{common::PositionType, coordinate_frame::{Client, ClientHandler}};

struct Logger;

#[async_trait::async_trait]
impl ClientHandler for Logger {
    async fn report_positions(&mut self, key_id: String, list: Vec<PositionType>, is_removed: bool) -> anyhow::Result<()> {
        tracing::info!(key_id, n = list.len(), is_removed);
        Ok(())
    }
    // every other operation defaults to a no-op
}

let (client, inbox) = Client::create(0, "gnss_sim")?;
client.connected().await?;
inbox.run(&mut Logger).await?;
```

### Building values

All structs derive `TypedBuilder`, `Default`, `Serialize`, `Deserialize` and `JsonSchema`:

```rust
let p = PositionType::builder()
    .ellipsoidal(EllipsoidalPositionType::builder().latitude(51.2).longitude(6.7).altitude(38.0).build())
    .coordinate_frame("WGS84")       // String fields take `impl Into<String>`
    .build();                         // Option fields take the value; `.altitude_opt(None)` for None
let json = serde_json::to_string(&p)?;                  // {"Ellipsoidal":{"Latitude":51.2,...}}
let schema = schemars::schema_for!(PositionType);       // e.g. to validate scenario files
```

## Linking at runtime

The SDK ships shared libraries, so the binary needs to find `.sdk/…/lib` at runtime:

* `just …` recipes set `LD_LIBRARY_PATH`, and the crate's own tests/examples get an rpath.
* In your own crate, add an rpath from a `build.rs`. `onboardapi` exports the directory as `DEP_ONBOARDAPI_RS_LIB_DIR`:

  ```rust
  fn main() {
      if let Ok(dir) = std::env::var("DEP_ONBOARDAPI_RS_LIB_DIR") {
          println!("cargo:rustc-link-arg=-Wl,-rpath,{dir}");
      }
  }
  ```
* The SDK lives elsewhere? Set `ONBOARDAPI_SDK_DIR` to the folder containing `include/` and `lib/`.

## Watching traffic with the onboardapi-inspector

```sh
just inspector          # terminal 1: starts the inspector (expects it next to this repo, or set ONBOARDAPI_INSPECTOR_DIR)
just inspector-demo     # terminal 2: M_CoordinateFrame service "demo_vehicle" + a Rust client, domain 1
```

* The inspector observes domain **1** by default, and the demo uses 1 too. Several domains can be entered comma-separated.
* Every `just` recipe exports `CYCLONEDDS_URI` for `config/cyclonedds/$DDS_PROFILE.xml` (default `localhost`). The inspector and the Rust processes must use the same profile:

| profile | use |
|---|---|
| `localhost.xml` | everything on one machine. Loopback only, unicast discovery to local ports |
| `wired.xml` | LAN. Set your adapter's IP (or `name="eth0"`) |
| `wifi.xml` | WiFi. Multicast only for participant discovery, the rest unicast |
| `unicast.xml` | no multicast. List every host under `<Peers>` |

  e.g. `DDS_PROFILE=wired just inspector-demo`, after editing the IP in `wired.xml`.
* `config/ddkit.json` (via `DDKIT_CFG`) sends ddkit warnings to stderr and info to `ddkit.log`.
* In the inspector, create a CoordinateFrame **client** on `demo_vehicle` and send e.g. `NotifyPosition`. The demo logs every incoming operation as JSON.
* Inspector 1.3.1 is built for onboardapi **9.9.0**. The demo uses `M_CoordinateFrame`, which did not change in 9.10.0. Modules extended in 9.10.0 (`M_Sensor` and its derived modules, `M_ObjectManager`, `M_FileTransfer`, `M_HidDisplay`, `M_Common` object types) may show the new fields or operations incompletely in the 9.9.0 inspector.

## CI and publishing

`.github/workflows/ci.yml` runs on every push and pull request:

| job | checks |
|---|---|
| codegen | `datamodel/*.rmodel` → generated code is committed and current |
| test (22.04, 24.04, arm64) | fmt, clippy `-D warnings`, all 56 tests over real DDS, two-process example run |
| package | `cargo package` builds both crates from their packaged sources |
| docs | rustdoc `-D warnings` in docs.rs mode (no SDK there) |
| msrv | `cargo check` on the `rust-version` toolchain |
| deny | licenses, RustSec advisories, crate sources |

The SDK is downloaded from the upstream release and cached per version and architecture. It never goes into the repository or the packages. The arm64 leg reports without blocking until it has passed once.

Release (`.github/workflows/release.yml`, on tags `rs-v*`): full CI → version/tag/repository checks → `cargo publish` (sys first) → GitHub release with the `.crate` files attached. A manual run of the workflow is a dry run.

One-time setup:

1. Create your GitHub repository, set `repository` in `Cargo.toml` to it, and push. Upstream stays available as a second remote for new `.rmodel` versions.
2. crates.io → Account Settings → API Tokens: create a token with `publish-new` + `publish-update`, then add it as repository secret `CARGO_REGISTRY_TOKEN`.
3. Settings → Environments → `crates-io`: optionally add yourself as a required reviewer, so every publish waits for a click.

Then each release:

```sh
just release 9.10.1     # bumps the version, runs lint + tests, commits, tags rs-v9.10.1
git push --follow-tags  # CI → publish → GitHub release
```

The crate version can move independently (wrapper fixes). The SDK version it builds against is `SDK_VERSION` in `crates/onboardapi-sys/build.rs`.

## Notes

* **Service names**: use `[a-zA-Z0-9_]` only. onboardapi maps other characters to `_` and warns.
* **Network interface**: set `CYCLONEDDS_URI` (see the onboardapi configuration docs) or Cyclone may pick the wrong NIC.
* **Inbox is unbounded**: drain it continuously, especially for best-effort `Stream*` operations.
* **Callbacks** run on onboardapi threads and are forwarded to the inbox, so handler code never blocks DDS. A Rust panic is never allowed to unwind into C++.
* **`connected()`** waits on tokio's blocking pool. Cancelling it does not cancel the underlying wait.
* **Licenses**: the SDK (`.sdk/`, EULA-RME-SDK-1.0) is git-ignored and never committed. The generated code derives from the EPL-2.0 `.rmodel` files.
