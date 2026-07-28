use std::any::type_name;

use swbt::model::{JoyConL as JoyConLModel, JoyConR as JoyConRModel, Pro};
use swbt::reporting::{Direct, Periodic};
use swbt::{
    Controller, ControllerBuilder, DirectJoyConL, DirectJoyConR, DirectProController, JoyConL,
    JoyConR, ProController,
};

fn assert_same_type<T, U>()
where
    T: Same<U>,
{
}

trait Same<T> {}
impl<T> Same<T> for T {}

fn assert_send<T: Send>() {}

#[test]
fn controller_aliases_are_the_six_model_reporting_combinations() {
    assert_same_type::<ProController, Controller<Pro, Periodic>>();
    assert_same_type::<DirectProController, Controller<Pro, Direct>>();
    assert_same_type::<JoyConL, Controller<JoyConLModel, Periodic>>();
    assert_same_type::<DirectJoyConL, Controller<JoyConLModel, Direct>>();
    assert_same_type::<JoyConR, Controller<JoyConRModel, Periodic>>();
    assert_same_type::<DirectJoyConR, Controller<JoyConRModel, Direct>>();

    assert_send::<ProController>();
    assert_send::<DirectProController>();
}

#[test]
fn controller_builder_keeps_both_type_axes() {
    assert_ne!(
        type_name::<ControllerBuilder<Pro, Periodic>>(),
        type_name::<ControllerBuilder<Pro, Direct>>()
    );
    assert_ne!(
        type_name::<ControllerBuilder<JoyConLModel, Periodic>>(),
        type_name::<ControllerBuilder<JoyConRModel, Periodic>>()
    );
}
