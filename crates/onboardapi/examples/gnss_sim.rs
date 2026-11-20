//! Simulated GNSS on `M_CoordinateFrame`.
//!
//! ```sh
//! cargo run --example gnss_sim -- service   # publishes a position at 10 Hz
//! cargo run --example gnss_sim -- client    # prints what it receives
//! ```
//!
//! Configuration via environment or `.env`: `ONBOARDAPI_DOMAIN` (default 0),
//! `GNSS_SERVICE_NAME` (default `gnss_sim`).

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Context;
use onboardapi::{
    common::{EllipsoidalPositionType, PositionType, PropertyAccessKind, TimeType},
    coordinate_frame::{
        Client, ClientHandler, CoordinateFrameDatumSupportType, CoordinateFrameSettingsType,
        CoordinateFrameStatusType, Service, ServiceMessage,
    },
};
use tracing::info;

const KEY: &str = "vehicle";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let raw = std::env::var("ONBOARDAPI_DOMAIN").unwrap_or_else(|_| "0".into());
    let domain: i32 = raw
        .trim()
        .parse()
        .with_context(|| format!("ONBOARDAPI_DOMAIN={raw:?} is not a DDS domain number"))?;
    let name = std::env::var("GNSS_SERVICE_NAME").unwrap_or_else(|_| "gnss_sim".into());

    match std::env::args().nth(1).as_deref() {
        Some("service") => run_service(domain, &name).await,
        Some("client") => run_client(domain, &name).await,
        _ => anyhow::bail!("usage: gnss_sim <service|client>"),
    }
}

async fn run_service(domain: i32, name: &str) -> anyhow::Result<()> {
    let (service, mut inbox) = Service::create(domain, name)?;
    info!(domain, name, "service up");

    service.report_coordinate_frame_settings(
        &CoordinateFrameSettingsType::builder()
            .supported_position_list(vec![
                CoordinateFrameDatumSupportType::builder()
                    .name(KEY)
                    .supported_coordinate_frame("WGS84")
                    .data_output_frequency(10.0)
                    .access_mode(PropertyAccessKind::ReadOnly)
                    .build(),
            ])
            .build(),
    )?;
    service.report_coordinate_frame_status(&CoordinateFrameStatusType::default(), false)?;

    let mut tick = tokio::time::interval(Duration::from_millis(100));
    let mut t = 0.0_f64;
    loop {
        tokio::select! {
            _ = tick.tick() => {
                t += 0.1;
                let position = PositionType::builder()
                    .ellipsoidal(EllipsoidalPositionType::builder()
                        .latitude(51.2277 + 0.001 * t.sin())
                        .longitude(6.7735 + 0.001 * t.cos())
                        .altitude(38.0)
                        .build())
                    .coordinate_frame("WGS84")
                    .timestamp(now())
                    .build();
                service.report_positions(KEY, &vec![position], false)?;
            }
            Some(msg) = inbox.recv() => match msg {
                ServiceMessage::NotifyPosition { key_id, position } => info!(key_id, ?position, "client set position"),
                other => info!(op = <&str>::from(&other), "ignoring"),
            },
            _ = tokio::signal::ctrl_c() => break,
        }
    }
    info!("shutting down; clients see IsRemoved=true");
    Ok(())
}

struct Printer;

#[async_trait::async_trait]
impl ClientHandler for Printer {
    async fn report_coordinate_frame_settings(
        &mut self,
        settings: CoordinateFrameSettingsType,
    ) -> anyhow::Result<()> {
        info!(settings = %serde_json::to_string(&settings)?, "settings");
        Ok(())
    }

    async fn report_positions(
        &mut self,
        key_id: String,
        positions: Vec<PositionType>,
        is_removed: bool,
    ) -> anyhow::Result<()> {
        if is_removed {
            info!(key_id, "positions removed");
        }
        for p in positions {
            if let Some(e) = p.ellipsoidal {
                info!(key_id, lat = e.latitude, lon = e.longitude, alt = ?e.altitude, "position");
            }
        }
        Ok(())
    }

    async fn report_coordinate_frame_status(
        &mut self,
        _status: CoordinateFrameStatusType,
        is_removed: bool,
    ) -> anyhow::Result<()> {
        info!(service_running = !is_removed, "status");
        Ok(())
    }
}

async fn run_client(domain: i32, name: &str) -> anyhow::Result<()> {
    let (client, inbox) = Client::create(domain, name)?;
    client.set_name("gnss_sim printer")?;
    info!(domain, name, "waiting for service");
    client.connected().await?;
    info!("connected");
    let mut printer = Printer;
    tokio::select! {
        r = inbox.run(&mut printer) => r,
        _ = tokio::signal::ctrl_c() => Ok(()),
    }
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
