//! Safe, async Rust wrapper for the Rheinmetall onboardapi 9.10.0.
//!
//! Every IDL module (`M_CoordinateFrame`, `M_Camera`, ...) is a Rust module
//! ([`coordinate_frame`], [`camera`], ...) containing
//!
//! * all data types as plain structs/enums (`serde`, `schemars`, `typed_builder`, `strum`),
//! * `ServiceMessage` / `ClientMessage`: one enum variant per operation,
//! * `ServiceHandler` / `ClientHandler`: async traits with a default no-op per operation,
//! * `Service` / `Client`: endpoints with one typed sender per outgoing operation.
//!
//! ```no_run
//! use onboardapi::coordinate_frame::{Client, ClientMessage, CoordinateFrameStatusType, Service, ServiceMessage};
//!
//! # async fn demo() -> onboardapi::Result<()> {
//! let (service, mut service_inbox) = Service::create(0, "gyro")?;
//! let (client, mut client_inbox) = Client::create(0, "gyro")?;
//! client.connected().await?;
//!
//! service.report_coordinate_frame_status(&CoordinateFrameStatusType::default(), false)?;
//! if let Some(ClientMessage::ReportCoordinateFrameStatus { status, is_removed }) = client_inbox.recv().await {
//!     println!("{status:?} removed={is_removed}");
//! }
//! # Ok(()) }
//! ```
//!
//! Callbacks arrive on onboardapi's threads and are forwarded into the endpoint's
//! [`Inbox`], so handlers run on your tokio runtime and never block the middleware.

mod endpoint;
mod error;
#[rustfmt::skip]
mod generated;
pub mod wire;

pub use endpoint::{ClientSide, Dispatch, Endpoint, Inbox, Message, Role};
pub use error::{Error, Result};
pub use generated::*;

/// Data model version these bindings were generated from.
pub const ONBOARDAPI_VERSION: &str = "9.10.0";
