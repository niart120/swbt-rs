use swbt::model::{HasDualSticks, HasLeftStick, HasRightStick, JoyConL, JoyConR, Pro};
use swbt::{ErrorKind, Stick};

fn assert_left_stick<M: HasLeftStick>() {}
fn assert_right_stick<M: HasRightStick>() {}
fn assert_dual_sticks<M: HasDualSticks>() {}

#[test]
fn model_markers_expose_their_declared_stick_capabilities() {
    assert_left_stick::<Pro>();
    assert_right_stick::<Pro>();
    assert_dual_sticks::<Pro>();
    assert_left_stick::<JoyConL>();
    assert_right_stick::<JoyConR>();
}

#[test]
fn stick_preserves_raw_boundaries_and_center() {
    assert_eq!(
        Stick::raw(Stick::MIN, Stick::MAX)
            .expect("boundary axes are valid")
            .axes(),
        (0, 4095)
    );
    assert_eq!(Stick::center().axes(), (2048, 2048));

    for (x, y) in [(4096, 0), (0, 4096)] {
        let error = Stick::raw(x, y).expect_err("axis must be rejected");
        assert_eq!(error.kind(), ErrorKind::InvalidInput);
    }
}

#[test]
fn normalized_stick_conversion_is_asymmetric_around_center() {
    assert_eq!(
        Stick::normalized(-1.0, -1.0)
            .expect("minimum normalized axes are valid")
            .axes(),
        (0, 0)
    );
    assert_eq!(
        Stick::normalized(0.0, 0.0)
            .expect("centered normalized axes are valid")
            .axes(),
        (2048, 2048)
    );
    assert_eq!(
        Stick::normalized(1.0, 1.0)
            .expect("maximum normalized axes are valid")
            .axes(),
        (4095, 4095)
    );
    assert_eq!(
        Stick::normalized(-0.5, 0.5)
            .expect("half tilt is valid")
            .axes(),
        (1024, 3072)
    );
}

#[test]
fn directional_helpers_validate_amount_and_preserve_the_other_axis() {
    assert_eq!(Stick::up(0.5).expect("valid amount").axes(), (2048, 3072));
    assert_eq!(Stick::down(0.5).expect("valid amount").axes(), (2048, 1024));
    assert_eq!(Stick::left(0.5).expect("valid amount").axes(), (1024, 2048));
    assert_eq!(
        Stick::right(0.5).expect("valid amount").axes(),
        (3072, 2048)
    );

    for amount in [-0.1, 1.1, f32::NAN, f32::INFINITY] {
        let error = Stick::up(amount).expect_err("amount must be rejected");
        assert_eq!(error.kind(), ErrorKind::InvalidInput);
    }
}

#[test]
fn normalized_input_rejects_non_finite_and_out_of_range_values() {
    for (x, y) in [
        (-1.1, 0.0),
        (1.1, 0.0),
        (f32::NAN, 0.0),
        (0.0, f32::NEG_INFINITY),
    ] {
        let error = Stick::normalized(x, y).expect_err("axis must be rejected");
        assert_eq!(error.kind(), ErrorKind::InvalidInput);
    }
}
