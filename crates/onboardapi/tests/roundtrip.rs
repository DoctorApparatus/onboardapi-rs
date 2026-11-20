//! End-to-end tests: a service and a client in one process, talking over DDS through the
//! real onboardapi libraries. Every payload goes Rust -> C++ -> DDS -> C++ -> Rust, so a
//! value that comes back equal proves both codecs and the type layouts agree.

use onboardapi::{
    common::{
        CartesianPositionType, EllipsoidalPositionType, PositionType, PropertyAccessKind,
        PropertyType, StorageKind, TimeType,
    },
    coordinate_frame::{
        self, CoordinateFrameDatumSupportType, CoordinateFrameSettingsType,
        CoordinateFrameStatusType,
    },
    laser_range_finder::{self, FirstOrLast},
};
use tokio::time::timeout;

mod support;

use support::{WAIT, domain, next, unique};

fn sample_position() -> PositionType {
    PositionType::builder()
        .cartesian(
            CartesianPositionType::builder()
                .x(1.25)
                .y(-2.5)
                .z(1e6)
                .build(),
        )
        .ellipsoidal(
            EllipsoidalPositionType::builder()
                .latitude(51.2)
                .longitude(6.78)
                .altitude(42.0)
                .build(),
        )
        .coordinate_frame("ECEF")
        .timestamp(
            TimeType::builder()
                .seconds(1_795_000_000)
                .nanoseconds(123_456_789)
                .build(),
        )
        .storage_mode(StorageKind::Persistent)
        .additional_info(vec![
            PropertyType::builder()
                .name("unicode")
                .str_value("Grüße ✓")
                .double_value(f64::MAX)
                .long_value(i32::MIN)
                .access_mode(PropertyAccessKind::ReadWrite)
                .build(),
        ])
        .build()
}

#[tokio::test(flavor = "multi_thread")]
async fn coordinate_frame_both_directions() -> anyhow::Result<()> {
    let name = unique("cf");
    let (service, mut service_inbox) = coordinate_frame::Service::create(domain(), &name)?;
    let (client, mut client_inbox) = coordinate_frame::Client::create(domain(), &name)?;
    client.set_name("rust roundtrip test")?;
    timeout(WAIT, client.connected()).await??;

    // service -> client, nested structs, lists, optionals and enums
    let settings = CoordinateFrameSettingsType::builder()
        .supported_position_list(vec![
            CoordinateFrameDatumSupportType::builder()
                .name("gnss")
                .supported_coordinate_frame("ECEF")
                .data_output_frequency(10.0)
                .access_mode(PropertyAccessKind::ReadOnly)
                .build(),
        ])
        .build();
    service.report_coordinate_frame_settings(&settings)?;
    match next(&mut client_inbox).await {
        coordinate_frame::ClientMessage::ReportCoordinateFrameSettings { settings: got } => {
            assert_eq!(got, settings)
        }
        other => panic!("unexpected {other:?}"),
    }

    // client -> service, keyed operation
    let position = sample_position();
    client.notify_position("turret", &position)?;
    match next(&mut service_inbox).await {
        coordinate_frame::ServiceMessage::NotifyPosition {
            key_id,
            position: got,
        } => {
            assert_eq!(key_id, "turret");
            assert_eq!(got, position);
        }
        other => panic!("unexpected {other:?}"),
    }

    // generic send path via the message enum
    service.send(&coordinate_frame::ClientMessage::ReportPositions {
        key_id: "turret".into(),
        position_list: vec![position.clone(), PositionType::default()],
        is_removed: false,
    })?;
    match next(&mut client_inbox).await {
        coordinate_frame::ClientMessage::ReportPositions {
            key_id,
            position_list,
            is_removed,
        } => {
            assert_eq!(key_id, "turret");
            assert_eq!(position_list, vec![position, PositionType::default()]);
            assert!(!is_removed);
        }
        other => panic!("unexpected {other:?}"),
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn late_joiner_gets_report_and_removal() -> anyhow::Result<()> {
    let name = unique("late");
    let (service, _service_inbox) = coordinate_frame::Service::create(domain(), &name)?;
    service.report_coordinate_frame_status(&CoordinateFrameStatusType::default(), false)?;

    // The client connects after the report was sent and still receives it.
    let (client, mut inbox) = coordinate_frame::Client::create(domain(), &name)?;
    timeout(WAIT, client.connected()).await??;
    match next(&mut inbox).await {
        coordinate_frame::ClientMessage::ReportCoordinateFrameStatus { is_removed, .. } => {
            assert!(!is_removed)
        }
        other => panic!("unexpected {other:?}"),
    }

    // Dropping the service marks the late-join data as removed.
    drop(service);
    loop {
        if let coordinate_frame::ClientMessage::ReportCoordinateFrameStatus {
            is_removed: true,
            ..
        } = next(&mut inbox).await
        {
            break;
        }
    }
    Ok(())
}

/// Inherited operations (`M_LaserRangeFinder::IService : M_Sensor::IService`) and the handler trait.
#[tokio::test(flavor = "multi_thread")]
async fn laser_range_finder_handler() -> anyhow::Result<()> {
    #[derive(Default)]
    struct Lrf {
        alive_checks: u32,
        range: Option<(Option<f64>, Option<f64>)>,
        first_or_last: Option<FirstOrLast>,
    }

    #[async_trait::async_trait]
    impl laser_range_finder::ServiceHandler for Lrf {
        async fn cmd_is_sensor_alive(&mut self) -> anyhow::Result<()> {
            self.alive_checks += 1;
            Ok(())
        }
        async fn config_set_user_range(
            &mut self,
            minimum: Option<f64>,
            maximum: Option<f64>,
        ) -> anyhow::Result<()> {
            self.range = Some((minimum, maximum));
            Ok(())
        }
        async fn config_set_first_or_last_measurement(
            &mut self,
            m: FirstOrLast,
        ) -> anyhow::Result<()> {
            self.first_or_last = Some(m);
            anyhow::bail!("stop after the last expected message")
        }
    }

    let name = unique("lrf");
    let (service, inbox) = laser_range_finder::Service::create(domain(), &name)?;
    let client = laser_range_finder::Client::create_write_only(domain(), &name)?;
    timeout(WAIT, client.connected()).await??;

    client.cmd_is_sensor_alive()?; // inherited from M_Sensor::IService
    client.config_set_user_range(&None, &Some(1500.0))?;
    client.config_set_first_or_last_measurement(FirstOrLast::Last)?;

    let mut lrf = Lrf::default();
    let err = timeout(WAIT, inbox.run(&mut lrf))
        .await?
        .expect_err("handler stops the loop");
    assert!(format!("{err:#}").contains("stop after"), "{err:#}");
    assert_eq!(lrf.alive_checks, 1);
    assert_eq!(lrf.range, Some((None, Some(1500.0))));
    assert_eq!(lrf.first_or_last, Some(FirstOrLast::Last));
    drop(service);
    Ok(())
}

#[test]
fn json_uses_idl_names() {
    let v = serde_json::to_value(sample_position()).expect("serializable");
    assert_eq!(v["CoordinateFrame"], "ECEF");
    assert_eq!(v["StorageMode"], "Persistent");
    assert_eq!(v["AdditionalInfo"][0]["AccessMode"], "Read_Write");
    let back: PositionType = serde_json::from_value(v).expect("deserializable");
    assert_eq!(back, sample_position());
}
