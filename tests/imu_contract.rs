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
