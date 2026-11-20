//! Traffic generator for the onboardapi-inspector.
//!
//! Starts an `M_CoordinateFrame` service `demo_vehicle` that drives in a circle and a
//! Rust client that sends a waypoint every few seconds. In the inspector you then see:
//!
//! * the service with its settings, status, positions and attitudes (10 Hz),
//! * the client's `NotifyPosition` commands,
//! * anything *you* send from the inspector (create a CoordinateFrame client there and
//!   call e.g. `NotifyPosition`): it is logged here as JSON.
//!
//! ```sh
//! just inspector          # terminal 1: inspector on the same DDS config
//! just inspector-demo     # terminal 2: this example
//! ```
//!
//! Environment (or `.env`): `ONBOARDAPI_DOMAIN` (default 1, the inspector's default),
//! `DEMO_SERVICE_NAME` (default `demo_vehicle`), `CYCLONEDDS_URI`.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Context;
use onboardapi::{
    common::{
        AttitudeType, EllipsoidalPositionType, EulerAttitudeType, OperationModeStateKind,
        OperationModeType, OperationStatusType, PositionType, PropertyAccessKind, StorageKind,
        TimeType,
    },
    coordinate_frame::{
        Client, CoordinateFrameDatumSupportType, CoordinateFrameSettingsType,
        CoordinateFrameStatusType, Service,
    },
};
use tracing::{info, warn};

const KEY: &str = "hull";
const CENTER: (f64, f64) = (51.2277, 6.7735); // Düsseldorf
const RADIUS_DEG: f64 = 0.002;
const RATE_HZ: f64 = 10.0;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let raw = std::env::var("ONBOARDAPI_DOMAIN").unwrap_or_else(|_| "1".into());
    let domain: i32 = raw
        .trim()
        .parse()
        .with_context(|| format!("ONBOARDAPI_DOMAIN={raw:?} is not a DDS domain number"))?;
    let name = std::env::var("DEMO_SERVICE_NAME").unwrap_or_else(|_| "demo_vehicle".into());
    match std::env::var("CYCLONEDDS_URI") {
        Ok(uri) => info!(uri, "cyclone config"),
        Err(_) => warn!(
            "CYCLONEDDS_URI not set; use `just inspector-demo` or config/cyclonedds/localhost.xml"
        ),
    }

    let service = tokio::spawn(run_service(domain, name.clone()));
    let client = tokio::spawn(run_client(domain, name.clone()));
    info!(
        domain,
        name, "running; open the inspector on domain {domain} — Ctrl+C to stop"
    );

    tokio::select! {
        r = service => r??,
        r = client => r??,
        _ = tokio::signal::ctrl_c() => info!("stopping; the inspector shows the service as removed"),
    }
    Ok(())
}

async fn run_service(domain: i32, name: String) -> anyhow::Result<()> {
    let (service, mut inbox) = Service::create(domain, &name)?;

    let datum = |frame: &str| {
        CoordinateFrameDatumSupportType::builder()
            .name(KEY)
            .supported_coordinate_frame(frame)
            .data_output_frequency(RATE_HZ)
            .access_mode(PropertyAccessKind::ReadWrite)
            .build()
    };
    service.report_coordinate_frame_settings(
        &CoordinateFrameSettingsType::builder()
            .supported_position_list(vec![datum("WGS84")])
            .supported_attitude_list(vec![datum("NED")])
            .build(),
    )?;
    service.report_coordinate_frame_status(&status(OperationModeStateKind::Simulation), false)?;

    let mut tick = tokio::time::interval(Duration::from_secs_f64(1.0 / RATE_HZ));
    let mut t = 0.0_f64;
    loop {
        tokio::select! {
            _ = tick.tick() => {
                t += 1.0 / RATE_HZ;
                let (position, attitude) = vehicle_at(t);
                service.report_positions(KEY, &vec![position], false)?;
                service.report_attitudes(KEY, &vec![attitude], false)?;
            }
            Some(msg) = inbox.recv() => {
                // Everything sent to the service (our client or the inspector) ends up here.
                info!(op = <&str>::from(&msg), json = %serde_json::to_string(&msg)?, "service received");
            }
        }
    }
}

async fn run_client(domain: i32, name: String) -> anyhow::Result<()> {
    let (client, mut inbox) = Client::create(domain, &name)?;
    client.set_name("rust inspector_demo client")?;
    client.connected().await?;
    info!("client connected to service");

    let mut every = tokio::time::interval(Duration::from_secs(3));
    let mut waypoint = 0u32;
    let mut reports = 0u64;
    loop {
        tokio::select! {
            _ = every.tick() => {
                waypoint += 1;
                let angle = f64::from(waypoint) * 0.7;
                let target = PositionType::builder()
                    .ellipsoidal(EllipsoidalPositionType::builder()
                        .latitude(CENTER.0 + RADIUS_DEG * angle.sin())
                        .longitude(CENTER.1 + RADIUS_DEG * angle.cos())
                        .build())
                    .coordinate_frame("WGS84")
                    .timestamp(now())
                    .storage_mode(StorageKind::NotPersistent)
                    .build();
                client.notify_position(KEY, &target)?;
                info!(waypoint, reports_received = reports, "client sent NotifyPosition");
            }
            Some(_) = inbox.recv() => reports += 1,
        }
    }
}

fn status(mode: OperationModeStateKind) -> CoordinateFrameStatusType {
    CoordinateFrameStatusType::builder()
        .operation_status(
            OperationStatusType::builder()
                .mode(
                    OperationModeType::builder()
                        .name("Operation")
                        .enumerated_value(mode)
                        .build(),
                )
                .build(),
        )
        .build()
}

/// Position on a circle plus a heading tangential to it.
fn vehicle_at(t: f64) -> (PositionType, AttitudeType) {
    let w = 0.1; // rad/s
    let (s, c) = (w * t).sin_cos();
    let position = PositionType::builder()
        .ellipsoidal(
            EllipsoidalPositionType::builder()
                .latitude(CENTER.0 + RADIUS_DEG * s)
                .longitude(CENTER.1 + RADIUS_DEG * c)
                .altitude(38.0)
                .build(),
        )
        .coordinate_frame("WGS84")
        .timestamp(now())
        .build();
    let heading = (90.0 - (w * t).to_degrees() + 180.0).rem_euclid(360.0);
    let attitude = AttitudeType::builder()
        .euler(
            EulerAttitudeType::builder()
                .roll(0.0)
                .pitch(1.5 * (t * 0.8).sin())
                .yaw(heading)
                .build(),
        )
        .coordinate_frame("NED")
        .timestamp(now())
        .build();
    (position, attitude)
}

fn now() -> TimeType {
    let d = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    TimeType::builder()
        .seconds(d.as_secs() as i64)
        .nanoseconds(d.subsec_nanos() as i32)
        .build()
}
