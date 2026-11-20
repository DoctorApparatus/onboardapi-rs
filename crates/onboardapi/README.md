# onboardapi (Rust)

Safe, async Rust bindings for the Rheinmetall **onboardapi** 9.10.0, a DDS-based service/client middleware. All 52 interface modules are covered, with 716 types and 485 operations. The bindings are generated from the open `.rmodel` data model and run on top of the official C++ SDK.

> Unofficial community bindings, not affiliated with or endorsed by Rheinmetall.

## Requirements

* Linux (x86_64; armv8/armv7 SDK builds are available too)
* A C++17 compiler (`g++`)
* The onboardapi C++ SDK from the [onboardapi releases](https://github.com/Rheinmetall/onboardapi/releases).
  It is licensed under EULA-RME-SDK-1.0 and **not** bundled. Point the build at it:

  ```sh
  export ONBOARDAPI_SDK_DIR=/path/to/sdk   # contains include/ and lib/
  ```

## Example

```rust
use onboardapi::coordinate_frame::{Client, ClientMessage, CoordinateFrameStatusType, Service};

async fn demo() -> onboardapi::Result<()> {
    let (service, _inbox) = Service::create(1, "gnss_sim")?;
    service.report_coordinate_frame_status(&CoordinateFrameStatusType::default(), false)?;

    let (client, mut inbox) = Client::create(1, "gnss_sim")?;
    client.connected().await?;
    while let Some(msg) = inbox.recv().await {
        if let ClientMessage::ReportCoordinateFrameStatus { is_removed, .. } = msg {
            println!("service running: {}", !is_removed);
        }
    }
    Ok(())
}
```

Each IDL module `M_Xyz` is the Rust module `onboardapi::xyz`. It contains the data types (`serde`, `schemars`, `typed_builder`, `strum`), `ServiceMessage`/`ClientMessage` enums, the async `ServiceHandler`/`ClientHandler` traits, and the `Service`/`Client` endpoints with one typed sender per operation.

## Runtime linking

The SDK ships shared libraries. Add an rpath in your binary's `build.rs`:

```rust,ignore
fn main() {
    if let Ok(dir) = std::env::var("DEP_ONBOARDAPI_RS_LIB_DIR") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,{dir}");
    }
}
```

or set `LD_LIBRARY_PATH`.

## License

EPL-2.0, like the `.rmodel` interface definitions it is generated from. The onboardapi runtime libraries are under their own EULA.
