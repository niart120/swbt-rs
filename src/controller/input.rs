use std::time::Duration;

use crate::{
    error::{Error, ErrorKind, Result},
    input::{Button, ButtonSet, InputState},
    model::ControllerModel,
};

pub(crate) const MAX_TAP_DURATION: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug)]
pub(crate) struct TapPlan<M: ControllerModel> {
    pressed: InputState<M>,
    released: InputState<M>,
    duration: Duration,
}

impl<M: ControllerModel> TapPlan<M> {
    pub(crate) fn into_parts(self) -> (InputState<M>, InputState<M>, Duration) {
        (self.pressed, self.released, self.duration)
    }
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T21 connects common press commands to the reporting-specific runtime policy"
    )
)]
pub(crate) fn press_candidate<M: ControllerModel>(
    current: &InputState<M>,
    buttons: impl IntoIterator<Item = Button<M>>,
) -> Result<InputState<M>> {
    let buttons = validated_buttons(buttons)?;
    Ok(press_validated(current, &buttons))
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T21 connects common release commands to the reporting-specific runtime policy"
    )
)]
pub(crate) fn release_candidate<M: ControllerModel>(
    current: &InputState<M>,
    buttons: impl IntoIterator<Item = Button<M>>,
) -> Result<InputState<M>> {
    let buttons = validated_buttons(buttons)?;
    Ok(release_validated(current, &buttons))
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T21 connects neutral commands to the reporting-specific runtime policy"
    )
)]
pub(crate) fn neutral_candidate<M: ControllerModel>() -> InputState<M> {
    InputState::neutral()
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T14 and T15 connect validated tap plans to each reporting policy"
    )
)]
pub(crate) fn tap_plan<M: ControllerModel>(
    current: &InputState<M>,
    buttons: impl IntoIterator<Item = Button<M>>,
    duration: Duration,
) -> Result<TapPlan<M>> {
    let buttons = validated_buttons(buttons)?;
    let duration = validated_tap_duration(duration)?;
    let pressed = press_validated(current, &buttons);
    let released = release_validated(&pressed, &buttons);
    Ok(TapPlan {
        pressed,
        released,
        duration,
    })
}

fn validated_buttons<M: ControllerModel>(
    buttons: impl IntoIterator<Item = Button<M>>,
) -> Result<ButtonSet<M>> {
    let buttons = buttons.into_iter().collect::<ButtonSet<_>>();
    if buttons.is_empty() {
        Err(Error::new(
            ErrorKind::InvalidInput,
            "button operation requires at least one button",
        ))
    } else {
        Ok(buttons)
    }
}

fn validated_tap_duration(duration: Duration) -> Result<Duration> {
    if duration <= MAX_TAP_DURATION {
        Ok(duration)
    } else {
        Err(Error::new(
            ErrorKind::InvalidInput,
            format!("tap duration must not exceed 24 hours: {duration:?}"),
        ))
    }
}

fn press_validated<M: ControllerModel>(
    current: &InputState<M>,
    buttons: &ButtonSet<M>,
) -> InputState<M> {
    let mut pressed = current.buttons().collect::<ButtonSet<_>>();
    for button in buttons.iter() {
        pressed.insert(button);
    }
    current.clone().with_buttons(pressed.iter())
}

fn release_validated<M: ControllerModel>(
    current: &InputState<M>,
    buttons: &ButtonSet<M>,
) -> InputState<M> {
    let retained = current
        .buttons()
        .filter(|button| !buttons.contains(*button))
        .collect::<ButtonSet<_>>();
    current.clone().with_buttons(retained.iter())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::{
        error::ErrorKind,
        input::{ImuFrame, InputState, ProButton, Stick},
        model::Pro,
        runtime::{
            periodic::commit_candidate as commit_periodic_candidate, state::InputStateStore,
        },
    };

    use super::{
        MAX_TAP_DURATION, neutral_candidate, press_candidate, release_candidate, tap_plan,
    };

    #[test]
    fn press_and_release_change_only_the_typed_button_set() {
        let a = ProButton::A;
        let b = ProButton::B;
        let x = ProButton::X;
        let y = ProButton::Y;
        let current = non_neutral_state().with_buttons([a, b]);

        let pressed = press_candidate(&current, [x, a, x]).expect("non-empty press is valid");
        assert_eq!(
            pressed,
            current.clone().with_buttons([a, b, x]),
            "press forms a duplicate-free union and preserves sticks and IMU"
        );

        let released =
            release_candidate(&pressed, [b, y, b, y]).expect("non-empty release is valid");
        assert_eq!(
            released,
            current.clone().with_buttons([a, x]),
            "release forms a set difference and ignores an already released button"
        );
    }

    #[test]
    fn neutral_candidate_resets_buttons_sticks_and_imu() {
        let current = non_neutral_state().with_buttons([ProButton::A]);
        let neutral = neutral_candidate::<Pro>();

        assert_ne!(neutral, current);
        assert_eq!(neutral, InputState::<Pro>::neutral());
    }

    #[test]
    fn empty_press_release_and_tap_are_invalid_input() {
        let current = non_neutral_state();

        let press_error = press_candidate(&current, []).expect_err("empty press must fail");
        let release_error = release_candidate(&current, []).expect_err("empty release must fail");
        let tap_error = tap_plan(&current, [], Duration::ZERO).expect_err("empty tap must fail");

        for error in [press_error, release_error, tap_error] {
            assert_eq!(error.kind(), ErrorKind::InvalidInput);
        }
    }

    #[test]
    fn tap_accepts_zero_and_24_hours_and_rejects_the_next_nanosecond() {
        let a = ProButton::A;
        let b = ProButton::B;
        let current = non_neutral_state().with_buttons([b]);

        for duration in [Duration::ZERO, MAX_TAP_DURATION] {
            let (pressed, released, validated_duration) = tap_plan(&current, [a], duration)
                .expect("inclusive tap duration boundary is valid")
                .into_parts();
            assert_eq!(pressed, current.clone().with_buttons([a, b]));
            assert_eq!(released, current);
            assert_eq!(validated_duration, duration);
        }

        let error = tap_plan(&current, [a], MAX_TAP_DURATION + Duration::from_nanos(1))
            .expect_err("duration above 24 hours must fail");
        assert_eq!(error.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn candidate_passes_to_the_periodic_commit_entry() {
        let current = non_neutral_state();
        let pressed = press_candidate(&current, [ProButton::A]).expect("non-empty press is valid");
        let mut periodic_store = InputStateStore::new();

        commit_periodic_candidate(pressed.clone(), &mut periodic_store);

        assert_eq!(periodic_store.snapshot(), pressed);
    }

    fn non_neutral_state() -> InputState<Pro> {
        InputState::neutral()
            .with_sticks(
                Stick::raw(0x123, 0x456).expect("valid left stick"),
                Stick::raw(0x789, 0xABC).expect("valid right stick"),
            )
            .with_imu([
                ImuFrame::raw([1, 2, 3], [4, 5, 6]),
                ImuFrame::raw([7, 8, 9], [10, 11, 12]),
                ImuFrame::raw([13, 14, 15], [16, 17, 18]),
            ])
    }
}
