use swbt::{ImuFrame, ImuSamples};

#[test]
fn imu_frame_preserves_signed_six_axis_boundaries() {
    let frame = ImuFrame::raw([i16::MIN, 0, i16::MAX], [i16::MAX, -1, i16::MIN]);

    assert_eq!(frame.accel(), [i16::MIN, 0, i16::MAX]);
    assert_eq!(frame.gyro(), [i16::MAX, -1, i16::MIN]);
}

#[test]
fn imu_frame_replaces_one_sensor_group_without_changing_the_other() {
    let neutral = ImuFrame::neutral();
    assert_eq!(neutral.accel(), [0; 3]);
    assert_eq!(neutral.gyro(), [0; 3]);

    let accelerated = neutral.with_accel([1, 2, 3]);
    assert_eq!(accelerated.accel(), [1, 2, 3]);
    assert_eq!(accelerated.gyro(), [0; 3]);

    let moving = accelerated.with_gyro([-1, -2, -3]);
    assert_eq!(moving.accel(), [1, 2, 3]);
    assert_eq!(moving.gyro(), [-1, -2, -3]);
}

#[test]
fn imu_samples_repeat_one_frame_or_preserve_three_frame_order() {
    let first = ImuFrame::raw([1, 0, 0], [0; 3]);
    let second = ImuFrame::raw([2, 0, 0], [0; 3]);
    let third = ImuFrame::raw([3, 0, 0], [0; 3]);

    assert_eq!(ImuSamples::from(first).into_frames(), [first; 3]);
    assert_eq!(
        ImuSamples::from([first, second, third]).into_frames(),
        [first, second, third]
    );
}

#[test]
fn physical_imu_values_use_the_python_calibration_scales() {
    let gyro = ImuFrame::gyro_rate(
        7.0_f64.to_radians(),
        (-14.0_f64).to_radians(),
        0.07_f64.to_radians(),
    )
    .unwrap();
    let accel = ImuFrame::accel_g(1.0, -0.5, 4.0).unwrap();

    assert_eq!(gyro.gyro(), [100, -200, 1]);
    assert_eq!(accel.accel(), [4096, -2048, 16384]);

    let rates = gyro.to_gyro_rate();
    assert!((rates[0] - 7.0_f64.to_radians()).abs() < 1.0e-12);
    assert!((rates[1] - (-14.0_f64).to_radians()).abs() < 1.0e-12);
    assert!((rates[2] - 0.07_f64.to_radians()).abs() < 1.0e-12);
    assert_eq!(accel.to_accel_g(), [1.0, -0.5, 4.0]);
}

#[test]
fn physical_imu_conversion_uses_ties_to_even() {
    let half_raw = 0.5 / 4096.0;
    let one_and_half_raw = 1.5 / 4096.0;

    assert_eq!(
        ImuFrame::accel_g(half_raw, one_and_half_raw, -half_raw)
            .unwrap()
            .accel(),
        [0, 2, 0]
    );
}

#[test]
fn physical_imu_conversion_rejects_non_finite_and_i16_overflow() {
    for result in [
        ImuFrame::accel_g(f64::NAN, 0.0, 0.0),
        ImuFrame::accel_g(0.0, f64::INFINITY, 0.0),
        ImuFrame::accel_g(8.0, 0.0, 0.0),
        ImuFrame::gyro_rate(f64::NEG_INFINITY, 0.0, 0.0),
        ImuFrame::gyro_rate(10_000.0, 0.0, 0.0),
    ] {
        assert_eq!(result.unwrap_err().kind(), swbt::ErrorKind::InvalidInput);
    }
}

#[test]
fn physical_with_methods_preserve_the_other_sensor_group() {
    let original = ImuFrame::raw([1, 2, 3], [4, 5, 6]);

    let with_accel = original.with_accel_g(1.0, -0.5, 4.0).unwrap();
    assert_eq!(with_accel.accel(), [4096, -2048, 16384]);
    assert_eq!(with_accel.gyro(), [4, 5, 6]);

    let with_gyro = original
        .with_gyro_rate(7.0_f64.to_radians(), 0.0, 0.0)
        .unwrap();
    assert_eq!(with_gyro.accel(), [1, 2, 3]);
    assert_eq!(with_gyro.gyro(), [100, 0, 0]);
}
