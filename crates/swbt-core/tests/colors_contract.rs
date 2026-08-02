use swbt_core::{ControllerColors, ErrorKind, Rgb24};

#[test]
fn rgb24_preserves_components_and_rejects_larger_integers() {
    let color = Rgb24::new(0x11, 0x22, 0x33);

    assert_eq!(color.components(), [0x11, 0x22, 0x33]);
    assert_eq!(color.value(), 0x112233);
    assert_eq!(Rgb24::try_from(0).unwrap().value(), 0);
    assert_eq!(Rgb24::try_from(0xFF_FFFF).unwrap().value(), 0xFF_FFFF);
    assert_eq!(
        Rgb24::try_from(0x100_0000).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
}

#[test]
fn controller_colors_keep_four_independent_rgb_values() {
    let colors = ControllerColors::new(
        Rgb24::new(0x11, 0x22, 0x33),
        Rgb24::new(0x44, 0x55, 0x66),
        Rgb24::new(0x77, 0x88, 0x99),
        Rgb24::new(0xAA, 0xBB, 0xCC),
    );

    assert_eq!(colors.body().value(), 0x112233);
    assert_eq!(colors.buttons().value(), 0x445566);
    assert_eq!(colors.left_grip().value(), 0x778899);
    assert_eq!(colors.right_grip().value(), 0xAABBCC);
    assert_eq!(
        colors.to_spi_bytes(),
        [
            0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC
        ]
    );
}

#[test]
fn controller_colors_match_the_python_default() {
    let colors = ControllerColors::default();

    assert_eq!(colors.body().value(), 0x323232);
    assert_eq!(colors.buttons().value(), 0xFFFFFF);
    assert_eq!(colors.left_grip().value(), 0x00B2FF);
    assert_eq!(colors.right_grip().value(), 0xFF3B30);
}
