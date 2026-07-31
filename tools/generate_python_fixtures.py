"""Generate deterministic pure-protocol fixtures from the pinned Python port."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import tomllib
from math import radians
from pathlib import Path
from typing import Any

SOURCE_REPOSITORY = "https://github.com/niart120/swbt-python"
SOURCE_COMMIT = "84d2723b127f70fc78e12f4496f5c40af0ccfb0a"
SOURCE_VERSION = "0.6.0"
GENERATOR = "tools/generate_python_fixtures.py"
SOURCE_PATHS = [
    "src/swbt/input.py",
    "src/swbt/imu.py",
    "src/swbt/protocol/buttons.py",
    "src/swbt/protocol/imu_report.py",
    "src/swbt/protocol/input_report.py",
    "src/swbt/protocol/output_report.py",
    "src/swbt/protocol/profiles/base.py",
    "src/swbt/protocol/profiles/joycon.py",
    "src/swbt/protocol/profiles/pro_controller.py",
    "src/swbt/protocol/session.py",
    "src/swbt/protocol/spi.py",
    "src/swbt/protocol/subcommand.py",
]
NEUTRAL_RUMBLE = bytes.fromhex("00 01 40 40 00 01 40 40")


def _git(repository: Path, *arguments: str) -> str:
    completed = subprocess.run(
        ["git", "-c", f"safe.directory={repository}", "-C", str(repository), *arguments],
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


def _expected_bytes(value: bytes, **decoded: object) -> dict[str, object]:
    return {
        "outcome": "bytes",
        "hex": value.hex(),
        "length": len(value),
        "decoded": decoded,
    }


def _case(
    case_id: str,
    kind: str,
    model: str,
    semantic_input: dict[str, object],
    expected: dict[str, object],
) -> dict[str, object]:
    return {
        "id": case_id,
        "kind": kind,
        "model": model,
        "input": semantic_input,
        "expected": expected,
    }


def _session_projection(session: Any) -> dict[str, object]:
    state = session.state
    return {
        "report_mode": state.report_mode,
        "report_mode_supported": state.report_mode_supported,
        "unsupported_report_mode": state.unsupported_report_mode,
        "player_lights": state.player_lights,
        "imu_mode": int(state.imu_mode),
        "vibration_enabled": state.vibration_enabled,
        "protocol_ready": state.protocol_ready,
    }


def _generate_cases() -> list[dict[str, object]]:
    from swbt.errors import ProtocolError
    from swbt.input import Button, IMUFrame, InputState, Stick
    from swbt.protocol.imu_report import ImuEncodingState, ImuMode, encode_imu_block
    from swbt.protocol.input_report import InputReportBuilder
    from swbt.protocol.output_report import OutputReportParser
    from swbt.protocol.profiles.joycon import JoyConLeftProfile, JoyConRightProfile
    from swbt.protocol.profiles.pro_controller import ProControllerProfile
    from swbt.protocol.session import SwitchHidSession
    from swbt.protocol.spi import VirtualSpiFlash
    from swbt.protocol.subcommand import SubcommandResponder

    profiles = {
        "pro": ProControllerProfile(),
        "joycon_l": JoyConLeftProfile(),
        "joycon_r": JoyConRightProfile(),
    }
    cases: list[dict[str, object]] = []

    normalized = [
        {
            "value": value,
            "raw": list(Stick.normalized(x=value, y=value).__dict__.values()),
        }
        for value in (-1.0, -0.5, 0.0, 0.5, 1.0)
    ]
    cases.append(
        _case(
            "conversion.stick.normalized",
            "conversion",
            "model-independent",
            {"values": [-1.0, -0.5, 0.0, 0.5, 1.0]},
            {"outcome": "values", "normalized": normalized},
        )
    )
    gyro = IMUFrame.gyro_rate(
        x_rad_s=radians(7.0),
        y_rad_s=radians(-14.0),
        z_rad_s=radians(0.07),
    )
    accel = IMUFrame.accel_g(x_g=1.0, y_g=-0.5, z_g=4.0)
    cases.append(
        _case(
            "conversion.imu.physical_scale",
            "conversion",
            "model-independent",
            {
                "gyro_dps": [7.0, -14.0, 0.07],
                "accel_g": [1.0, -0.5, 4.0],
            },
            {
                "outcome": "values",
                "gyro_raw": [gyro.gyro_x, gyro.gyro_y, gyro.gyro_z],
                "accel_raw": [accel.accel_x, accel.accel_y, accel.accel_z],
            },
        )
    )

    for model, profile in profiles.items():
        report = InputReportBuilder(profile).build_0x30(InputState.neutral())
        cases.append(
            _case(
                f"input.{model}.neutral",
                "input_report",
                model,
                {"timer": 0, "buttons": [], "sticks": "neutral", "imu": "neutral"},
                _expected_bytes(
                    report,
                    report_id=report[0],
                    timer=report[1],
                    battery_connection=report[2],
                    button_hex=report[3:6].hex(),
                    left_stick_hex=report[6:9].hex(),
                    right_stick_hex=report[9:12].hex(),
                    vibrator=report[12],
                    imu_hex=report[13:49].hex(),
                ),
            )
        )

        buttons = tuple(profile.button_bits)
        report = InputReportBuilder(profile).build_0x30(
            InputState.neutral().with_buttons(buttons),
            timer=0x42,
        )
        cases.append(
            _case(
                f"input.{model}.all_buttons",
                "input_report",
                model,
                {"timer": 0x42, "buttons": sorted(button.name for button in buttons)},
                _expected_bytes(report, button_hex=report[3:6].hex()),
            )
        )

    custom_sticks = InputState.neutral().with_sticks(
        left_stick=Stick.raw(x=0x123, y=0xABC),
        right_stick=Stick.raw(x=0xFFF, y=0x000),
    )
    report = InputReportBuilder(profiles["pro"]).build_0x30(custom_sticks)
    cases.append(
        _case(
            "input.pro.custom_sticks",
            "input_report",
            "pro",
            {"left": [0x123, 0xABC], "right": [0xFFF, 0x000]},
            _expected_bytes(
                report,
                left_stick_hex=report[6:9].hex(),
                right_stick_hex=report[9:12].hex(),
            ),
        )
    )

    distinct_frames = (
        IMUFrame.raw(accel=(1, -2, 3), gyro=(-4, 5, -6)),
        IMUFrame.raw(accel=(7, -8, 9), gyro=(-10, 11, -12)),
        IMUFrame.raw(accel=(13, -14, 15), gyro=(-16, 17, -18)),
    )
    distinct_state = InputState.neutral().with_imu(*distinct_frames)
    for model, profile in profiles.items():
        report = InputReportBuilder(profile).build_0x30(distinct_state)
        cases.append(
            _case(
                f"input.{model}.standard_imu",
                "input_report",
                model,
                {
                    "imu_mode": 1,
                    "frames": [
                        [1, -2, 3, -4, 5, -6],
                        [7, -8, 9, -10, 11, -12],
                        [13, -14, 15, -16, 17, -18],
                    ],
                },
                _expected_bytes(report, imu_hex=report[13:49].hex()),
            )
        )

    quaternion_frames = (
        IMUFrame.raw(accel=(1, 2, 3), gyro=(0, 0, 1000)),
        IMUFrame.raw(accel=(4, 5, 6), gyro=(0, 0, 1000)),
        IMUFrame.raw(accel=(7, 8, 9), gyro=(0, 0, 1000)),
    )
    for model, profile in profiles.items():
        for mode in range(2, 6):
            encoded = encode_imu_block(
                state=ImuEncodingState(previous_report_ns=0),
                mode=ImuMode(mode),
                frames=quaternion_frames,
                gyro_calibration=profile.gyro_calibration,
                now_ns=1_000_000_000,
            )
            report = InputReportBuilder(profile).build_0x30(
                InputState.neutral().with_imu(*quaternion_frames),
                imu_block=encoded.block,
            )
            cases.append(
                _case(
                    f"input.{model}.quaternion_mode_{mode:02x}",
                    "input_report",
                    model,
                    {
                        "imu_mode": mode,
                        "previous_report_ns": 0,
                        "now_ns": 1_000_000_000,
                        "frames": [
                            [1, 2, 3, 0, 0, 1000],
                            [4, 5, 6, 0, 0, 1000],
                            [7, 8, 9, 0, 0, 1000],
                        ],
                    },
                    _expected_bytes(report, imu_hex=encoded.block.hex()),
                )
            )

    parser = OutputReportParser()
    parser_inputs = {
        "output.valid_01": bytes.fromhex("01 ab 00 01 40 40 00 01 40 40 03 30"),
        "output.valid_10": bytes.fromhex("10 2a 00 01 40 40 00 01 40 40"),
    }
    for case_id, raw in parser_inputs.items():
        parsed = parser.parse(raw)
        cases.append(
            _case(
                case_id,
                "output_report",
                "model-independent",
                {"raw_hex": raw.hex()},
                {
                    "outcome": "parsed",
                    "report_id": parsed.report_id,
                    "packet_id": parsed.packet_id,
                    "rumble_hex": parsed.rumble.hex() if parsed.rumble is not None else None,
                    "subcommand_id": parsed.subcommand_id,
                    "payload_hex": parsed.subcommand_payload.hex(),
                },
            )
        )
    for case_id, raw in {
        "output.error.empty": b"",
        "output.error.unknown": b"\x99",
        "output.error.truncated_01": bytes.fromhex("01 ab 00 01 40 40 00 01 40 40"),
        "output.error.truncated_10": bytes.fromhex("10 2a 00 01 40 40 00 01 40"),
    }.items():
        try:
            parser.parse(raw)
        except ProtocolError as error:
            expected = {"outcome": "error", "error_type": type(error).__name__}
        else:
            raise AssertionError(f"{case_id} unexpectedly parsed")
        cases.append(
            _case(
                case_id,
                "output_report",
                "model-independent",
                {"raw_hex": raw.hex()},
                expected,
            )
        )

    for model, profile in profiles.items():
        spi = VirtualSpiFlash(profile=profile)
        for suffix, address, size in (
            ("device_type", 0x6012, 1),
            ("calibration", 0x6020, 24),
            ("colors", 0x6050, 12),
            ("erased", 0x70000, 2),
        ):
            value = spi.read(address, size)
            cases.append(
                _case(
                    f"spi.{model}.{suffix}",
                    "spi",
                    model,
                    {"address": address, "size": size},
                    _expected_bytes(value),
                )
            )

    def subcommand_case(
        case_id: str,
        model: str,
        subcommand_id: int,
        payload: bytes = b"",
    ) -> None:
        profile = profiles[model]
        session = SwitchHidSession(profile)
        responder = SubcommandResponder(
            profile=profile,
            device_info_bluetooth_address=bytes.fromhex("00 1b dc f9 9f 7d"),
        )
        raw = bytes((0x01, 0x0A)) + NEUTRAL_RUMBLE + bytes((subcommand_id,)) + payload
        parsed = parser.parse(raw)
        reply = responder.respond(
            parsed,
            state=InputState.neutral(),
            session=session,
        )
        cases.append(
            _case(
                case_id,
                "subcommand",
                model,
                {
                    "subcommand_id": subcommand_id,
                    "payload_hex": payload.hex(),
                    "bluetooth_address_hex": "001bdcf99f7d",
                },
                {
                    **_expected_bytes(
                        reply,
                        ack=reply[13],
                        reply_to=reply[14],
                        data_hex=reply[15:].hex(),
                    ),
                    "session": _session_projection(session),
                },
            )
        )

    for model in profiles:
        subcommand_case(f"subcommand.{model}.device_info", model, 0x02)
    subcommand_case("subcommand.pro.report_mode", "pro", 0x03, b"\x30")
    subcommand_case("subcommand.pro.unsupported_report_mode", "pro", 0x03, b"\x3f")
    subcommand_case("subcommand.pro.trigger_elapsed", "pro", 0x04)
    subcommand_case("subcommand.joycon_l.trigger_elapsed", "joycon_l", 0x04)
    subcommand_case("subcommand.pro.simple_ack", "pro", 0x08)
    subcommand_case(
        "subcommand.pro.spi_device_type",
        "pro",
        0x10,
        bytes.fromhex("12 60 00 00 01"),
    )
    subcommand_case("subcommand.pro.mcu_config", "pro", 0x21, b"\x01")
    subcommand_case("subcommand.pro.player_lights", "pro", 0x30, b"\x01")
    subcommand_case("subcommand.pro.imu_mode", "pro", 0x40, b"\x02")
    subcommand_case("subcommand.pro.vibration", "pro", 0x48, b"\x01")

    return cases


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--python-repo", type=Path, required=True)
    parser.add_argument(
        "--output",
        type=Path,
        default=Path("tests/fixtures/python-v0.6.0/protocol/protocol-fixtures.json"),
    )
    arguments = parser.parse_args()
    repository = arguments.python_repo.resolve()
    source_tree = _verify_source(repository)

    source_root = (repository / "src").resolve()
    sys.path.insert(0, str(source_root))
    cases = _generate_cases()

    imported_bumble = sorted(
        name for name in sys.modules if name == "bumble" or name.startswith("bumble.")
    )
    if imported_bumble:
        raise SystemExit(f"pure protocol fixture generation imported Bumble: {imported_bumble}")

    input_module = sys.modules["swbt.input"]
    if not Path(input_module.__file__).resolve().is_relative_to(source_root):
        raise SystemExit("swbt was not imported from the requested Python checkout")

    document = {
        "format": "swbt.protocol-fixtures",
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
