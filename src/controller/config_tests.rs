use std::{path::PathBuf, time::Duration};

use crate::{
    AdapterSelector,
    error::ErrorKind,
    model,
    profile::{ControllerColors, Rgb24},
    reporting,
};

use super::{
    BuilderConfig, DirectJoyConL, DirectJoyConR, DirectProController, JoyConL, JoyConR,
    ProController,
};

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

#[test]
fn transport_config_projects_model_protocol_identity() {
    let configurations = [
        (
            ProController::builder("usb:0")
                .validate()
                .expect("Pro builder")
                .finalize_with_profile(|_| unreachable!("ephemeral controller"))
                .expect("Pro controller config")
                .transport_config(),
            "Pro Controller",
        ),
        (
            JoyConL::builder("usb:0")
                .validate()
                .expect("Joy-Con L builder")
                .finalize_with_profile(|_| unreachable!("ephemeral controller"))
                .expect("Joy-Con L controller config")
                .transport_config(),
            "Joy-Con (L)",
        ),
        (
            JoyConR::builder("usb:0")
                .validate()
                .expect("Joy-Con R builder")
                .finalize_with_profile(|_| unreachable!("ephemeral controller"))
                .expect("Joy-Con R controller config")
                .transport_config(),
            "Joy-Con (R)",
        ),
    ];

    for (configuration, local_name) in configurations {
        assert_eq!(configuration.local_name(), local_name);
        assert_eq!(configuration.class_of_device(), 0x002508);
    }
}

#[test]
fn transport_config_is_identical_for_periodic_and_direct_reporting() {
    let pro_periodic = ProController::builder("usb:0")
        .validate()
        .expect("periodic Pro builder")
        .finalize_with_profile(|_| unreachable!("ephemeral controller"))
        .expect("periodic Pro config");
    let pro_direct = DirectProController::builder("usb:0")
        .validate()
        .expect("direct Pro builder")
        .finalize_with_profile(|_| unreachable!("ephemeral controller"))
        .expect("direct Pro config");
    let joycon_l_periodic = JoyConL::builder("usb:0")
        .validate()
        .expect("periodic Joy-Con L builder")
        .finalize_with_profile(|_| unreachable!("ephemeral controller"))
        .expect("periodic Joy-Con L config");
    let joycon_l_direct = DirectJoyConL::builder("usb:0")
        .validate()
        .expect("direct Joy-Con L builder")
        .finalize_with_profile(|_| unreachable!("ephemeral controller"))
        .expect("direct Joy-Con L config");
    let joycon_r_periodic = JoyConR::builder("usb:0")
        .validate()
        .expect("periodic Joy-Con R builder")
        .finalize_with_profile(|_| unreachable!("ephemeral controller"))
        .expect("periodic Joy-Con R config");
    let joycon_r_direct = DirectJoyConR::builder("usb:0")
        .validate()
        .expect("direct Joy-Con R builder")
        .finalize_with_profile(|_| unreachable!("ephemeral controller"))
        .expect("direct Joy-Con R config");

    assert_eq!(
        pro_periodic.transport_config(),
        pro_direct.transport_config()
    );
    assert_eq!(
        joycon_l_periodic.transport_config(),
        joycon_l_direct.transport_config()
    );
    assert_eq!(
        joycon_r_periodic.transport_config(),
        joycon_r_direct.transport_config()
    );
}

fn assert_unit_reporting_state(config: &BuilderConfig<model::Pro, reporting::Direct>) {
    let _: &() = config.mode_config();
}
