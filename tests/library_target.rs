use swbt as _;

#[test]
fn package_exposes_the_swbt_library_contract() {
    assert_eq!(env!("CARGO_PKG_NAME"), "swbt-rs");
    assert_eq!(env!("CARGO_PKG_RUST_VERSION"), "1.87");
    assert_eq!(env!("CARGO_PKG_LICENSE"), "MIT");
}
