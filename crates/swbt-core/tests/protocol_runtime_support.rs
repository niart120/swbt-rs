use std::mem::size_of;

use swbt_core::{__private::SwitchHidProtocol, Pro};

#[test]
fn runtime_support_exposes_the_single_core_protocol_engine() {
    assert!(size_of::<SwitchHidProtocol<Pro>>() > 0);
}
