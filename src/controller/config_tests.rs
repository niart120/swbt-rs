use std::{path::PathBuf, time::Duration};

use crate::{
    AdapterSelector,
    error::ErrorKind,
    model,
    profile::{ControllerColors, Rgb24},
    reporting,
};

use super::{BuilderConfig, DirectProController, JoyConL, ProController};

#[test]
fn periodic_builder_retains_settings_and_defaults_to_eight_milliseconds() {
    let profile_path = PathBuf::from("profiles/custom-pro.json");
    let colors = ControllerColors::new(
        Rgb24::new(0x01, 0x02, 0x03),
        Rgb24::new(0x04, 0x05, 0x06),
        Rgb24::new(0x07, 0x08, 0x09),
        Rgb24::new(0x0A, 0x0B, 0x0C),
    );

    let config = ProController::builder("usb:0")
        .profile_path(profile_path.clone())
        .controller_colors(colors)
        .validate()
        .expect("default periodic configuration must be valid");

    assert_eq!(config.adapter(), &AdapterSelector::from("usb:0"));
    assert_eq!(config.profile_path(), Some(profile_path.as_path()));
    assert_eq!(config.colors(), colors);
    assert_eq!(config.report_period(), Duration::from_millis(8));
}

#[test]
fn periodic_builder_uses_the_selected_models_default_colors() {
    let config = JoyConL::builder("usb:left")
        .validate()
        .expect("model defaults must be valid");

    assert_eq!(
        config.colors(),
        ControllerColors::new(
            Rgb24::new(0x00, 0xB2, 0xFF),
            Rgb24::new(0x32, 0x32, 0x32),
            Rgb24::new(0x00, 0xB2, 0xFF),
            Rgb24::new(0x00, 0xB2, 0xFF),
        )
    );
}

#[test]
fn periodic_report_period_accepts_inclusive_bounds_and_rejects_outside_values() {
    for period in [Duration::from_millis(1), Duration::from_secs(1)] {
        let config = ProController::builder("usb:0")
            .report_period(period)
            .validate()
            .expect("inclusive report-period boundary must be valid");

        assert_eq!(config.report_period(), period);
    }

    for period in [
        Duration::ZERO,
        Duration::from_millis(1) - Duration::from_nanos(1),
        Duration::from_secs(1) + Duration::from_nanos(1),
    ] {
        let error = ProController::builder("usb:0")
            .report_period(period)
            .validate()
            .expect_err("out-of-range report period must fail validation");

        assert_eq!(error.kind(), ErrorKind::InvalidInput);
    }
}

#[test]
fn direct_config_uses_unit_reporting_state() {
    let config = DirectProController::builder(AdapterSelector::from("usb:1"))
        .validate()
        .expect("direct configuration must be valid");

    assert_eq!(config.adapter(), &AdapterSelector::from("usb:1"));
    assert_unit_reporting_state(&config);
}

fn assert_unit_reporting_state(config: &BuilderConfig<model::Pro, reporting::Direct>) {
    let _: &() = config.mode_config();
}
