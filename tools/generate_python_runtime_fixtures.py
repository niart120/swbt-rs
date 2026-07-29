"""Generate deterministic runtime-semantics fixtures from the pinned Python port."""

from __future__ import annotations

import argparse
import asyncio
import json
import subprocess
import sys
import tomllib
from pathlib import Path
from typing import Any

SOURCE_REPOSITORY = "https://github.com/niart120/swbt-python"
SOURCE_COMMIT = "84d2723b127f70fc78e12f4496f5c40af0ccfb0a"
SOURCE_VERSION = "0.6.0"
GENERATOR = "tools/generate_python_runtime_fixtures.py"
SOURCE_PATHS = [
    "src/swbt/diagnostics.py",
    "src/swbt/errors.py",
    "src/swbt/gamepad/_config.py",
    "src/swbt/gamepad/connection.py",
    "src/swbt/gamepad/output.py",
    "src/swbt/gamepad/protocol_handshake.py",
    "src/swbt/gamepad/runtime.py",
    "src/swbt/imu.py",
    "src/swbt/input.py",
    "src/swbt/protocol/buttons.py",
    "src/swbt/protocol/imu_report.py",
    "src/swbt/protocol/input_report.py",
    "src/swbt/protocol/output_report.py",
    "src/swbt/protocol/profiles/base.py",
    "src/swbt/protocol/profiles/pro_controller.py",
    "src/swbt/protocol/session.py",
    "src/swbt/protocol/spi.py",
    "src/swbt/protocol/subcommand.py",
    "src/swbt/report_loop.py",
    "src/swbt/state_store.py",
    "src/swbt/transport/base.py",
    "src/swbt/transport/fake.py",
]
OUTPUT_REPORT_PREFIX = bytes.fromhex("01 00 00 00 00 00 00 00 00 00")


def _git(repository: Path, *arguments: str) -> str:
    completed = subprocess.run(
        [
            "git",
            "-c",
            f"safe.directory={repository}",
            "-C",
            str(repository),
            *arguments,
        ],
        check=True,
        capture_output=True,
        text=True,
    )
    return completed.stdout.strip()


def _verify_source(repository: Path) -> str:
    if sys.version_info[:2] != (3, 13):
        raise SystemExit(
            "fixture generation requires Python 3.13; "
            f"got {sys.version_info.major}.{sys.version_info.minor}"
        )
    if _git(repository, "rev-parse", "HEAD") != SOURCE_COMMIT:
        raise SystemExit(f"Python checkout must be at {SOURCE_COMMIT}")
    if _git(repository, "status", "--porcelain"):
        raise SystemExit("Python checkout must be clean")

    project = tomllib.loads((repository / "pyproject.toml").read_text(encoding="utf-8"))
    if project["project"]["version"] != SOURCE_VERSION:
        raise SystemExit(f"Python package version must be {SOURCE_VERSION}")
    return _git(repository, "rev-parse", f"{SOURCE_COMMIT}^{{tree}}")


def _case(
    case_id: str,
    *,
    classification: str,
    reporting: str,
    steps: list[dict[str, object]],
    checkpoints: list[dict[str, object]],
) -> dict[str, object]:
    return {
        "id": case_id,
        "classification": classification,
        "model": "pro",
        "reporting": reporting,
        "steps": steps,
        "expected": {"checkpoints": checkpoints},
    }


async def _generate_cases() -> list[dict[str, object]]:
    from swbt.diagnostics import DiagnosticsRecorder
    from swbt.gamepad._config import _GamepadConfig
    from swbt.gamepad.output import OutputReportDispatcher
    from swbt.gamepad.protocol_handshake import ProtocolHandshake
    from swbt.gamepad.runtime import ControllerRuntime
    from swbt.input import Button, IMUFrame, InputState
    from swbt.protocol.input_report import InputReportBuilder
    from swbt.protocol.profiles.pro_controller import default_controller_profile
    from swbt.protocol.session import SwitchHidSession
    from swbt.report_loop import ReportLoop, ReportSender
    from swbt.state_store import InputStateStore
    from swbt.transport.fake import FakeHidTransport

    class FixtureSendError(RuntimeError):
        pass

    class DeterministicClock:
        def __init__(self) -> None:
            self.now_ns = 0
            self.sleep_delays_ns: list[int] = []

        def clock_ns(self) -> int:
            return self.now_ns

        def time(self) -> float:
            return self.now_ns / 1_000_000_000

        def advance_ns(self, duration_ns: int) -> None:
            self.now_ns += duration_ns

        async def sleep(self, delay: float) -> None:
            delay_ns = round(delay * 1_000_000_000)
            self.sleep_delays_ns.append(delay_ns)
            self.advance_ns(delay_ns)
            await asyncio.sleep(0)

    class TrackingTransport(FakeHidTransport):
        def __init__(self, clock: DeterministicClock | None = None) -> None:
            super().__init__()
            self.clock = clock or DeterministicClock()
            self.attempts: list[dict[str, object]] = []
            self._armed_report_id: int | None = None
            self._armed_occurrence = 0
            self._matching_attempts = 0

        def reject_matching(self, report_id: int, *, occurrence: int = 1) -> None:
            self._armed_report_id = report_id
            self._armed_occurrence = occurrence
            self._matching_attempts = 0

        async def send_interrupt(self, payload: bytes) -> None:
            reject = False
            if payload and payload[0] == self._armed_report_id:
                self._matching_attempts += 1
                reject = self._matching_attempts == self._armed_occurrence
            attempt = {
                "sequence": len(self.attempts),
                "payload": bytes(payload),
                "attempted_at_ns": self.clock.clock_ns(),
                "result": "rejected" if reject else "accepted",
            }
            self.attempts.append(attempt)
            if reject:
                self._armed_report_id = None
                raise FixtureSendError("scripted transport rejection")
            await super().send_interrupt(payload)
            attempt["completed_at_ns"] = self.clock.clock_ns()

    class BlockingStateStore(InputStateStore):
        def __init__(self, initial_state: Any) -> None:
            super().__init__(initial_state)
            self.snapshot_entered = asyncio.Event()
            self.release_snapshot = asyncio.Event()
            self._block_next_snapshot = True

        async def snapshot(self) -> Any:
            if self._block_next_snapshot:
                self._block_next_snapshot = False
                self.snapshot_entered.set()
                await self.release_snapshot.wait()
            return await super().snapshot()

    profile = default_controller_profile()

    def input_projection(state: Any) -> dict[str, object]:
        return {
            "buttons": sorted(button.name for button in state.buttons),
            "left_stick": [state.left_stick.x, state.left_stick.y],
            "right_stick": [state.right_stick.x, state.right_stick.y],
            "imu_frames": [
                [
                    frame.accel_x,
                    frame.accel_y,
                    frame.accel_z,
                    frame.gyro_x,
                    frame.gyro_y,
                    frame.gyro_z,
                ]
                for frame in state.imu_frames
            ],
        }

    def session_projection(session: Any) -> dict[str, object]:
        state = session.state
        return {
            "report_mode": state.report_mode,
            "report_mode_supported": state.report_mode_supported,
            "player_lights": state.player_lights,
            "imu_mode": int(state.imu_mode),
            "imu_previous_report_ns": state.imu_encoding_state.previous_report_ns,
            "vibration_enabled": state.vibration_enabled,
            "observed_subcommands": sorted(state.observed_subcommands),
            "protocol_ready": state.protocol_ready,
        }

    def report_projection(attempt: dict[str, object]) -> dict[str, object]:
        payload = attempt["payload"]
        assert isinstance(payload, bytes)
        report_id = payload[0]
        projection: dict[str, object] = {
            "sequence": attempt["sequence"],
            "report_id": report_id,
            "timer": payload[1] if report_id in (0x21, 0x30) else None,
            "button_hex": payload[3:6].hex() if len(payload) >= 6 else None,
            "result": attempt["result"],
            "attempted_at_ns": attempt["attempted_at_ns"],
        }
        if "completed_at_ns" in attempt:
            projection["completed_at_ns"] = attempt["completed_at_ns"]
        if report_id == 0x21 and len(payload) >= 15:
            projection["subcommand_id"] = payload[14]
        if report_id == 0x30 and len(payload) >= 49:
            projection["imu_hex"] = payload[13:49].hex()
        return projection

    def report_projections(
        transport: TrackingTransport,
        *,
        start: int = 0,
    ) -> list[dict[str, object]]:
        return [report_projection(attempt) for attempt in transport.attempts[start:]]

    async def make_runtime(
        reporting: str,
        transport: TrackingTransport,
    ) -> tuple[ControllerRuntime, DeterministicClock]:
        clock = transport.clock
        runtime = ControllerRuntime(
            _GamepadConfig(
                profile=profile,
                report_period_us=10_000_000,
            ),
            reporting_mode=reporting,
            transport=transport,
            _report_sender_monotonic_time=clock.time,
            _report_sender_sleep=clock.sleep,
        )
        await runtime.open()
        return runtime, clock

    async def connect_protocol_ready(
        runtime: ControllerRuntime,
        transport: TrackingTransport,
    ) -> list[dict[str, object]]:
        start = len(transport.attempts)
        await transport.connect()
        bootstrap_reports = await transport.wait_for_interrupt_report_count(1)
        if bootstrap_reports[-1][0] != 0x30:
            raise AssertionError(
                "protocol handshake did not start with bootstrap input"
            )
        await transport.inject_interrupt_data(
            OUTPUT_REPORT_PREFIX + bytes.fromhex("03 30")
        )
        await transport.inject_interrupt_data(
            OUTPUT_REPORT_PREFIX + bytes.fromhex("30 01")
        )
        for _ in range(20):
            if runtime.status().connection_state == "connected":
                break
            await asyncio.sleep(0)
        if runtime.status().connection_state != "connected":
            raise AssertionError("fake runtime did not become protocol-ready")
        return report_projections(transport, start=start)

    cases: list[dict[str, object]] = []

    clock = DeterministicClock()
    transport = TrackingTransport(clock)
    await transport.open()
    session = SwitchHidSession(profile)
    sender = ReportSender(
        transport=transport,
        input_report_builder=InputReportBuilder(profile),
        session=session,
        clock_ns=clock.clock_ns,
        monotonic_time=clock.time,
    )
    reply = bytearray(50)
    reply[0] = 0x21
    reply[14] = 0x02
    await sender.send_input(InputState.neutral(), reason="input")
    await sender.send_subcommand_reply(lambda: bytes(reply))
    await sender.send_input(InputState.neutral(), reason="input")
    cases.append(
        _case(
            "sender.shared_timer",
            classification="parity",
            reporting="model-independent",
            steps=[
                {
                    "id": "input_0",
                    "action": "send_input",
                    "buttons": [],
                    "acceptance": "accepted",
                },
                {
                    "id": "reply_1",
                    "action": "send_reply",
                    "subcommand_id": 0x02,
                    "acceptance": "accepted",
                },
                {
                    "id": "input_2",
                    "action": "send_input",
                    "buttons": [],
                    "acceptance": "accepted",
                },
            ],
            checkpoints=[
                {
                    "id": "shared_sequence",
                    "after_step": "input_2",
                    "report_attempts": report_projections(transport),
                    "next_timer": sender._timer,
                }
            ],
        )
    )

    async def prefix_case(ready: bool) -> dict[str, object]:
        local_clock = DeterministicClock()
        local_transport = TrackingTransport(local_clock)
        await local_transport.open()
        local_session = SwitchHidSession(profile)
        if ready:
            local_session.set_report_mode(0x30, supported=True)
            local_session.set_player_lights(0x01)
        state_store = InputStateStore(InputState.neutral().with_buttons([Button.A]))
        local_sender = ReportSender(
            transport=local_transport,
            input_report_builder=InputReportBuilder(profile),
            session=local_session,
            clock_ns=local_clock.clock_ns,
            monotonic_time=local_clock.time,
        )
        dispatcher = OutputReportDispatcher(
            diagnostics=DiagnosticsRecorder(),
            require_reply_sender=lambda: None,
            send_subcommand_reply=local_sender.send_subcommand_reply,
            session=local_session,
            state_store=state_store,
        )
        await dispatcher.dispatch(OUTPUT_REPORT_PREFIX + bytes.fromhex("02"))
        return {
            "report_attempts": report_projections(local_transport),
            "committed_input": input_projection(state_store.current),
            "protocol_session": session_projection(local_session),
        }

    cases.append(
        _case(
            "sender.pre_ready_neutral_prefix",
            classification="parity",
            reporting="model-independent",
            steps=[
                {
                    "id": "seed_input",
                    "action": "commit_input",
                    "buttons": ["A"],
                },
                {
                    "id": "device_info",
                    "action": "dispatch_subcommand",
                    "subcommand_id": 0x02,
                    "pre_transition_ready": False,
                    "acceptance": "accepted",
                },
            ],
            checkpoints=[
                {
                    "id": "neutral_prefix",
                    "after_step": "device_info",
                    **await prefix_case(False),
                }
            ],
        )
    )
    cases.append(
        _case(
            "sender.ready_current_prefix",
            classification="parity",
            reporting="model-independent",
            steps=[
                {
                    "id": "seed_input",
                    "action": "commit_input",
                    "buttons": ["A"],
                },
                {
                    "id": "seed_ready_session",
                    "action": "set_protocol_session",
                    "report_mode": 0x30,
                    "player_lights": 0x01,
                    "protocol_ready": True,
                },
                {
                    "id": "device_info",
                    "action": "dispatch_subcommand",
                    "subcommand_id": 0x02,
                    "pre_transition_ready": True,
                    "acceptance": "accepted",
                },
            ],
            checkpoints=[
                {
                    "id": "current_prefix",
                    "after_step": "device_info",
                    **await prefix_case(True),
                }
            ],
        )
    )

    clock = DeterministicClock()
    transport = TrackingTransport(clock)
    await transport.open()
    session = SwitchHidSession(profile)
    state_store = InputStateStore()
    sender = ReportSender(
        transport=transport,
        input_report_builder=InputReportBuilder(profile),
        session=session,
        clock_ns=clock.clock_ns,
        monotonic_time=clock.time,
    )
    accepted_state = InputState.neutral().with_buttons([Button.A])
    await sender.send_input(
        accepted_state,
        reason="direct",
        commit_state_store=state_store,
    )
    cases.append(
        _case(
            "direct.accepted_commit",
            classification="parity",
            reporting="direct",
            steps=[
                {
                    "id": "accepted_input",
                    "action": "send_direct",
                    "buttons": ["A"],
                    "acceptance": "accepted",
                }
            ],
            checkpoints=[
                {
                    "id": "committed",
                    "after_step": "accepted_input",
                    "report_attempts": report_projections(transport),
                    "next_timer": sender._timer,
                    "committed_input": input_projection(state_store.current),
                }
            ],
        )
    )

    rejected_state = InputState.neutral().with_buttons([Button.X])
    transport.reject_matching(0x30)
    try:
        await sender.send_input(
            rejected_state,
            reason="direct",
            commit_state_store=state_store,
        )
    except FixtureSendError:
        pass
    else:
        raise AssertionError("scripted direct rejection did not fail")
    rejected_checkpoint = {
        "id": "rejected",
        "after_step": "rejected_input",
        "report_attempts": report_projections(transport),
        "next_timer": sender._timer,
        "committed_input": input_projection(state_store.current),
    }
    await sender.send_input(
        rejected_state,
        reason="direct",
        commit_state_store=state_store,
    )
    cases.append(
        _case(
            "direct.rejected_no_commit",
            classification="parity",
            reporting="direct",
            steps=[
                {
                    "id": "accepted_seed",
                    "action": "send_direct",
                    "buttons": ["A"],
                    "acceptance": "accepted",
                },
                {
                    "id": "rejected_input",
                    "action": "send_direct",
                    "buttons": ["X"],
                    "acceptance": "rejected",
                },
                {
                    "id": "retry_input",
                    "action": "send_direct",
                    "buttons": ["X"],
                    "acceptance": "accepted",
                },
            ],
            checkpoints=[
                rejected_checkpoint,
                {
                    "id": "retry_committed",
                    "after_step": "retry_input",
                    "report_attempts": report_projections(transport),
                    "next_timer": sender._timer,
                    "committed_input": input_projection(state_store.current),
                },
            ],
        )
    )

    clock = DeterministicClock()
    transport = TrackingTransport(clock)
    await transport.open()
    session = SwitchHidSession(profile)
    state_store = BlockingStateStore(
        InputState.neutral().with_imu(IMUFrame.accel(z=4096))
    )
    sender = ReportSender(
        transport=transport,
        input_report_builder=InputReportBuilder(profile),
        session=session,
        clock_ns=clock.clock_ns,
        monotonic_time=clock.time,
    )
    report_loop = ReportLoop(
        transport=transport,
        state_store=state_store,
        input_report_builder=InputReportBuilder(profile),
        session=session,
        sender=sender,
    )
    periodic = asyncio.create_task(report_loop.send_current_input())
    await state_store.snapshot_entered.wait()

    def enable_quaternion_and_build_ack() -> bytes:
        session.set_imu_mode(0x02)
        value = bytearray(50)
        value[0] = 0x21
        value[14] = 0x40
        return bytes(value)

    acknowledgement = asyncio.create_task(
        report_loop.send_subcommand_reply(enable_quaternion_and_build_ack)
    )
    await asyncio.sleep(0)
    state_store.release_snapshot.set()
    await periodic
    await acknowledgement
    await report_loop.send_current_input()
    cases.append(
        _case(
            "sender.imu_mode_inflight_order",
            classification="parity",
            reporting="periodic",
            steps=[
                {
                    "id": "seed_input",
                    "action": "commit_input",
                    "buttons": [],
                    "imu_frames": [
                        {"accel": [0, 0, 4096], "gyro": [0, 0, 0]},
                    ],
                },
                {
                    "id": "old_mode_snapshot",
                    "action": "block_input_snapshot",
                },
                {
                    "id": "imu_request",
                    "action": "queue_subcommand_reply",
                    "subcommand_id": 0x40,
                },
                {
                    "id": "release_snapshot",
                    "action": "release_input_snapshot",
                },
                {
                    "id": "new_mode_input",
                    "action": "send_current_input",
                },
            ],
            checkpoints=[
                {
                    "id": "accepted_order",
                    "after_step": "new_mode_input",
                    "report_attempts": report_projections(transport),
                    "next_timer": sender._timer,
                    "protocol_session": session_projection(session),
                }
            ],
        )
    )

    clock = DeterministicClock()
    transport = TrackingTransport(clock)
    await transport.open()
    session = SwitchHidSession(profile)
    session.set_report_mode(0x30, supported=True)
    session.set_player_lights(0x01)
    state_store = InputStateStore(InputState.neutral().with_buttons([Button.A]))
    sender = ReportSender(
        transport=transport,
        input_report_builder=InputReportBuilder(profile),
        session=session,
        clock_ns=clock.clock_ns,
        monotonic_time=clock.time,
    )
    dispatcher = OutputReportDispatcher(
        diagnostics=DiagnosticsRecorder(),
        require_reply_sender=lambda: None,
        send_subcommand_reply=sender.send_subcommand_reply,
        session=session,
        state_store=state_store,
    )
    enable_quaternion = OUTPUT_REPORT_PREFIX + bytes.fromhex("40 02")
    transport.reject_matching(0x21)
    try:
        await dispatcher.dispatch(enable_quaternion)
    except FixtureSendError:
        pass
    else:
        raise AssertionError("scripted 0x40 reply rejection did not fail")
    failure_checkpoint = {
        "id": "reply_rejected",
        "after_step": "rejected_reply",
        "report_attempts": report_projections(transport),
        "next_timer": sender._timer,
        "automatic_holdoff_until_ns": round(
            sender._automatic_input_holdoff_until * 1_000_000_000
        ),
        "protocol_session": session_projection(session),
    }
    await dispatcher.dispatch(enable_quaternion)
    automatic_during_holdoff = await sender.send_automatic_input(
        state_store.current,
        reason="periodic",
    )
    holdoff_checkpoint = {
        "id": "accepted_reply_holds_off_periodic",
        "after_step": "heldoff_input",
        "report_attempts": report_projections(transport),
        "next_timer": sender._timer,
        "automatic_input_sent": automatic_during_holdoff,
        "automatic_holdoff_until_ns": round(
            sender._automatic_input_holdoff_until * 1_000_000_000
        ),
        "protocol_session": session_projection(session),
    }
    clock.advance_ns(300_000_000)
    automatic_at_boundary = await sender.send_automatic_input(
        state_store.current,
        reason="periodic",
    )
    cases.append(
        _case(
            "subcommand.imu_mode_rejected_reply",
            classification="parity",
            reporting="model-independent",
            steps=[
                {
                    "id": "seed_input",
                    "action": "commit_input",
                    "buttons": ["A"],
                },
                {
                    "id": "seed_ready_session",
                    "action": "set_protocol_session",
                    "report_mode": 0x30,
                    "player_lights": 0x01,
                    "protocol_ready": True,
                },
                {
                    "id": "rejected_reply",
                    "action": "dispatch_subcommand",
                    "subcommand_id": 0x40,
                    "acceptance": "rejected",
                },
                {
                    "id": "accepted_retry",
                    "action": "dispatch_subcommand",
                    "subcommand_id": 0x40,
                    "acceptance": "accepted",
                },
                {
                    "id": "heldoff_input",
                    "action": "send_automatic_input",
                    "acceptance": "suppressed",
                },
                {
                    "id": "holdoff_boundary",
                    "action": "advance_clock",
                    "duration_ns": 300_000_000,
                },
                {
                    "id": "new_mode_input",
                    "action": "send_automatic_input",
                    "acceptance": "accepted",
                },
            ],
            checkpoints=[
                failure_checkpoint,
                holdoff_checkpoint,
                {
                    "id": "retry_then_input",
                    "after_step": "new_mode_input",
                    "report_attempts": report_projections(transport),
                    "next_timer": sender._timer,
                    "automatic_input_sent": automatic_at_boundary,
                    "automatic_holdoff_until_ns": round(
                        sender._automatic_input_holdoff_until * 1_000_000_000
                    ),
                    "protocol_session": session_projection(session),
                },
            ],
        )
    )

    transport = TrackingTransport()
    runtime, _ = await make_runtime("periodic", transport)
    await runtime.press(Button.A)
    preconnection_state = input_projection(runtime.snapshot())
    handshake_reports = await connect_protocol_ready(runtime, transport)
    start = len(transport.attempts)
    await runtime._send_current_input()
    first_current_report = report_projections(transport, start=start)
    await runtime.close(neutral=False)
    cases.append(
        _case(
            "periodic.pre_connection_update",
            classification="baseline_observation",
            reporting="periodic",
            steps=[
                {
                    "id": "open_runtime",
                    "action": "open_runtime",
                    "connection_state": "opened",
                },
                {
                    "id": "preconnection_press",
                    "action": "press",
                    "buttons": ["A"],
                },
                {"id": "connect_ready", "action": "complete_protocol_handshake"},
                {"id": "first_current", "action": "send_current_input"},
            ],
            checkpoints=[
                {
                    "id": "python_carries_state",
                    "after_step": "first_current",
                    "preconnection_input": preconnection_state,
                    "handshake_report_attempts": handshake_reports,
                    "first_current_report_attempts": first_current_report,
                }
            ],
        )
    )

    transport = TrackingTransport()
    runtime, _ = await make_runtime("periodic", transport)
    await connect_protocol_ready(runtime, transport)
    await runtime.press(Button.A)
    state_before_disconnect = input_projection(runtime.snapshot())
    await transport.disconnect(reason=0x13)
    cases.append(
        _case(
            "periodic.disconnect_neutralize",
            classification="parity",
            reporting="periodic",
            steps=[
                {
                    "id": "connect_ready",
                    "action": "complete_protocol_handshake",
                },
                {
                    "id": "press",
                    "action": "press",
                    "buttons": ["A"],
                },
                {
                    "id": "disconnect",
                    "action": "transport_disconnect",
                    "reason": 0x13,
                },
            ],
            checkpoints=[
                {
                    "id": "neutralized",
                    "after_step": "disconnect",
                    "before_disconnect": state_before_disconnect,
                    "committed_input": input_projection(runtime.snapshot()),
                    "connection_state": runtime.status().connection_state,
                }
            ],
        )
    )

    async def tap_rejection_case(reporting: str) -> dict[str, object]:
        local_transport = TrackingTransport()
        local_runtime, _ = await make_runtime(reporting, local_transport)
        await connect_protocol_ready(local_runtime, local_transport)
        await local_runtime.press(Button.ZL)
        start = len(local_transport.attempts)
        local_transport.reject_matching(0x30, occurrence=2)
        transport_rejection_propagated = False
        try:
            await local_runtime.tap(Button.A, duration=0)
        except FixtureSendError:
            transport_rejection_propagated = True
        else:
            raise AssertionError("scripted tap release rejection did not fail")
        result = {
            "report_attempts": report_projections(local_transport, start=start),
            "committed_input": input_projection(local_runtime.snapshot()),
            "transport_rejection_propagated": transport_rejection_propagated,
        }
        await local_runtime.close(neutral=False)
        return result

    cases.append(
        _case(
            "tap.periodic_release_rejected",
            classification="parity",
            reporting="periodic",
            steps=[
                {
                    "id": "connect_ready",
                    "action": "complete_protocol_handshake",
                },
                {
                    "id": "seed_held_input",
                    "action": "commit_periodic_input",
                    "buttons": ["ZL"],
                },
                {
                    "id": "press_report",
                    "action": "tap_press",
                    "buttons": ["A"],
                    "acceptance": "accepted",
                },
                {
                    "id": "release_report",
                    "action": "tap_release",
                    "buttons": ["A"],
                    "acceptance": "rejected",
                },
            ],
            checkpoints=[
                {
                    "id": "released_state_retained",
                    "after_step": "release_report",
                    **await tap_rejection_case("periodic"),
                }
            ],
        )
    )
    cases.append(
        _case(
            "tap.direct_release_rejected",
            classification="parity",
            reporting="direct",
            steps=[
                {
                    "id": "connect_ready",
                    "action": "complete_protocol_handshake",
                },
                {
                    "id": "seed_held_input",
                    "action": "send_direct",
                    "buttons": ["ZL"],
                    "acceptance": "accepted",
                },
                {
                    "id": "press_report",
                    "action": "tap_press",
                    "buttons": ["A"],
                    "acceptance": "accepted",
                },
                {
                    "id": "release_report",
                    "action": "tap_release",
                    "buttons": ["A"],
                    "acceptance": "rejected",
                },
            ],
            checkpoints=[
                {
                    "id": "pressed_state_retained",
                    "after_step": "release_report",
                    **await tap_rejection_case("direct"),
                }
            ],
        )
    )

    clock = DeterministicClock()
    transport = TrackingTransport(clock)
    await transport.open()
    session = SwitchHidSession(profile)
    session.set_imu_mode(0x02)
    state_store = InputStateStore()
    sender = ReportSender(
        transport=transport,
        input_report_builder=InputReportBuilder(profile),
        session=session,
        clock_ns=clock.clock_ns,
        monotonic_time=clock.time,
    )
    seed = InputState.neutral().with_buttons([Button.A]).with_imu(IMUFrame.gyro(z=1000))
    rejected = (
        InputState.neutral().with_buttons([Button.X]).with_imu(IMUFrame.gyro(z=2000))
    )
    clock.advance_ns(1_000_000_000)
    await sender.send_input(seed, reason="direct", commit_state_store=state_store)
    accepted_session = session_projection(session)
    transport.reject_matching(0x30)
    clock.advance_ns(1_000_000_000)
    try:
        await sender.send_input(
            rejected,
            reason="direct",
            commit_state_store=state_store,
        )
    except FixtureSendError:
        pass
    else:
        raise AssertionError("scripted quaternion input rejection did not fail")
    rejection_checkpoint = {
        "id": "python_advances_before_acceptance",
        "after_step": "rejected_input",
        "report_attempts": report_projections(transport),
        "next_timer": sender._timer,
        "committed_input": input_projection(state_store.current),
        "protocol_session": session_projection(session),
    }
    await sender.send_input(
        rejected,
        reason="direct",
        commit_state_store=state_store,
    )
    rejected_attempt = transport.attempts[-2]["payload"]
    retry_attempt = transport.attempts[-1]["payload"]
    cases.append(
        _case(
            "imu.rejected_quaternion_input",
            classification="baseline_observation",
            reporting="direct",
            steps=[
                {
                    "id": "enable_quaternion_mode",
                    "action": "set_imu_mode",
                    "imu_mode": 0x02,
                },
                {
                    "id": "accepted_seed",
                    "action": "send_direct",
                    "buttons": ["A"],
                    "imu_gyro_z": 1000,
                    "at_ns": 1_000_000_000,
                    "acceptance": "accepted",
                },
                {
                    "id": "rejected_input",
                    "action": "send_direct",
                    "buttons": ["X"],
                    "imu_gyro_z": 2000,
                    "at_ns": 2_000_000_000,
                    "acceptance": "rejected",
                },
                {
                    "id": "retry_same_time",
                    "action": "send_direct",
                    "buttons": ["X"],
                    "imu_gyro_z": 2000,
                    "at_ns": 2_000_000_000,
                    "acceptance": "accepted",
                },
            ],
            checkpoints=[
                {
                    "id": "accepted_epoch",
                    "after_step": "accepted_seed",
                    "protocol_session": accepted_session,
                },
                rejection_checkpoint,
                {
                    "id": "same_time_retry",
                    "after_step": "retry_same_time",
                    "report_attempts": report_projections(transport),
                    "next_timer": sender._timer,
                    "committed_input": input_projection(state_store.current),
                    "protocol_session": session_projection(session),
                    "retry_wire_matches_rejected": retry_attempt == rejected_attempt,
                },
            ],
        )
    )

    class SlowSender:
        def __init__(self, clock: DeterministicClock) -> None:
            self.clock = clock
            self.sends: list[dict[str, object]] = []

        async def send_automatic_input(self, _state: Any, *, reason: str) -> bool:
            started_at_ns = self.clock.clock_ns()
            self.clock.advance_ns(250_000_000)
            self.sends.append(
                {
                    "reason": reason,
                    "started_at_ns": started_at_ns,
                    "completed_at_ns": self.clock.clock_ns(),
                }
            )
            return True

    class ProbeHandshake(ProtocolHandshake):
        def __init__(
            self, *args: Any, clock: DeterministicClock, **kwargs: Any
        ) -> None:
            super().__init__(*args, **kwargs)
            self.clock = clock
            self.waits_ns: list[int] = []

        async def _wait_for_change(self, wait_seconds: float | None) -> None:
            if wait_seconds is None:
                raise AssertionError("bootstrap probe expected a finite retry")
            wait_ns = round(wait_seconds * 1_000_000_000)
            self.waits_ns.append(wait_ns)
            self.clock.advance_ns(wait_ns)
            if len(self.waits_ns) == 2:
                self._stopped = True

    clock = DeterministicClock()
    slow_sender = SlowSender(clock)
    handshake = ProbeHandshake(
        sender=slow_sender,
        session=SwitchHidSession(profile),
        report_period_us=8_000,
        on_outcome=lambda _outcome: None,
        bootstrap_retry_seconds=1.0,
        clock=clock,
    )
    await handshake._run()
    cases.append(
        _case(
            "handshake.retry_after_send_latency",
            classification="baseline_observation",
            reporting="model-independent",
            steps=[
                {
                    "id": "start_not_ready",
                    "action": "start_handshake",
                    "protocol_ready": False,
                    "bootstrap_retry_ns": 1_000_000_000,
                },
                {
                    "id": "first_bootstrap",
                    "action": "send_bootstrap",
                    "latency_ns": 250_000_000,
                },
                {
                    "id": "relative_wait",
                    "action": "wait_after_completion",
                    "duration_ns": 1_000_000_000,
                },
                {
                    "id": "second_bootstrap",
                    "action": "send_bootstrap",
                    "latency_ns": 250_000_000,
                },
                {
                    "id": "second_relative_wait",
                    "action": "wait_after_completion",
                    "duration_ns": 1_000_000_000,
                },
            ],
            checkpoints=[
                {
                    "id": "python_relative_retry",
                    "after_step": "second_relative_wait",
                    "send_attempts": slow_sender.sends,
                    "requested_waits_ns": handshake.waits_ns,
                    "second_start_minus_first_start_ns": (
                        slow_sender.sends[1]["started_at_ns"]
                        - slow_sender.sends[0]["started_at_ns"]
                    ),
                    "second_start_minus_first_completion_ns": (
                        slow_sender.sends[1]["started_at_ns"]
                        - slow_sender.sends[0]["completed_at_ns"]
                    ),
                }
            ],
        )
    )

    return sorted(cases, key=lambda case: str(case["id"]))


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--python-repo", type=Path, required=True)
    parser.add_argument(
        "--output",
        type=Path,
        default=Path("tests/fixtures/python-v0.6.0/runtime/runtime-semantics.json"),
    )
    arguments = parser.parse_args()
    repository = arguments.python_repo.resolve()
    source_tree = _verify_source(repository)

    source_root = (repository / "src").resolve()
    sys.path.insert(0, str(source_root))
    cases = asyncio.run(_generate_cases())

    imported_bumble = sorted(
        name for name in sys.modules if name == "bumble" or name.startswith("bumble.")
    )
    if imported_bumble:
        raise SystemExit(
            f"fake runtime fixture generation imported Bumble: {imported_bumble}"
        )

    report_loop_module = sys.modules["swbt.report_loop"]
    if not Path(report_loop_module.__file__).resolve().is_relative_to(source_root):
        raise SystemExit("swbt was not imported from the requested Python checkout")

    document = {
        "format": "swbt.runtime-semantics",
        "schema_version": 1,
        "source_repository": SOURCE_REPOSITORY,
        "source_commit": SOURCE_COMMIT,
        "source_tree": source_tree,
        "source_version": SOURCE_VERSION,
        "python_version": "3.13",
        "generator": GENERATOR,
        "source_paths": SOURCE_PATHS,
        "cases": cases,
    }
    output = arguments.output.resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(
        json.dumps(document, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
        newline="\n",
    )
    print(f"wrote {len(cases)} cases to {output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
