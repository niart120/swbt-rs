"""Generate HID descriptor and SDP policy fixtures from the pinned Python port."""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import sys
import tomllib
from pathlib import Path
from typing import Any

SOURCE_REPOSITORY = "https://github.com/niart120/swbt-python"
SOURCE_COMMIT = "84d2723b127f70fc78e12f4496f5c40af0ccfb0a"
SOURCE_VERSION = "0.6.0"
GENERATOR = "tools/generate_python_hid_service_fixtures.py"
SOURCE_PATHS = [
    "src/swbt/protocol/descriptors.py",
    "src/swbt/protocol/profiles/base.py",
    "src/swbt/protocol/profiles/joycon.py",
    "src/swbt/protocol/profiles/pro_controller.py",
    "src/swbt/transport/_bumble_sdp.py",
]


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


def _policy(profile: Any) -> dict[str, object]:
    policy = profile.hid_sdp_policy
    return {
        "service_name": policy.service_name or profile.device_name,
        "service_description": policy.service_description,
        "provider_name": policy.provider_name,
        "device_release_number": policy.device_release_number,
        "bluetooth_profile_version": policy.bluetooth_profile_version,
        "parser_version": policy.parser_version,
        "device_subclass": policy.device_subclass,
        "country_code": policy.country_code,
        "virtual_cable": policy.virtual_cable,
        "reconnect_initiate": policy.reconnect_initiate,
        "remote_wake": policy.remote_wake,
        "profile_version": policy.profile_version,
        "supervision_timeout": policy.supervision_timeout,
        "normally_connectable": policy.normally_connectable,
        "boot_device": policy.boot_device,
        "ssr_host_max_latency": policy.ssr_host_max_latency,
        "ssr_host_min_timeout": policy.ssr_host_min_timeout,
    }


def _generate_models() -> tuple[dict[str, object], dict[str, object]]:
    from swbt.protocol.profiles.joycon import JoyConLeftProfile, JoyConRightProfile
    from swbt.protocol.profiles.pro_controller import ProControllerProfile

    profiles = {
        "pro": ProControllerProfile(),
        "joycon_l": JoyConLeftProfile(),
        "joycon_r": JoyConRightProfile(),
    }
    descriptors = {profile.hid_report_descriptor for profile in profiles.values()}
    if len(descriptors) != 1:
        raise SystemExit("the pinned M4 baseline must use one shared HID descriptor")
    descriptor = descriptors.pop()
    descriptor_document = {
        "length": len(descriptor),
        "sha256": hashlib.sha256(descriptor).hexdigest(),
        "hex": descriptor.hex(),
    }
    models = {
        model: {
            "local_name": profile.device_name,
            "sdp_policy": _policy(profile),
        }
        for model, profile in profiles.items()
    }
    return descriptor_document, models


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--python-repo", type=Path, required=True)
    parser.add_argument(
        "--output",
        type=Path,
        default=Path(
            "tests/fixtures/python-v0.6.0/hid/hid-service-fixtures.json"
        ),
    )
    arguments = parser.parse_args()
    repository = arguments.python_repo.resolve()
    source_tree = _verify_source(repository)

    source_root = (repository / "src").resolve()
    sys.path.insert(0, str(source_root))
    descriptor, models = _generate_models()

    imported_bumble = sorted(
        name for name in sys.modules if name == "bumble" or name.startswith("bumble.")
    )
    if imported_bumble:
        raise SystemExit(f"HID service fixture generation imported Bumble: {imported_bumble}")

    profile_module = sys.modules["swbt.protocol.profiles.base"]
    if not Path(profile_module.__file__).resolve().is_relative_to(source_root):
        raise SystemExit("swbt was not imported from the requested Python checkout")

    document = {
        "format": "swbt.hid-service-fixtures",
        "schema_version": 1,
        "source_repository": SOURCE_REPOSITORY,
        "source_commit": SOURCE_COMMIT,
        "source_tree": source_tree,
        "source_version": SOURCE_VERSION,
        "python_version": "3.13",
        "generator": GENERATOR,
        "source_paths": SOURCE_PATHS,
        "descriptor": descriptor,
        "models": models,
    }
    output = arguments.output.resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(
        json.dumps(document, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
        newline="\n",
    )
    print(f"wrote HID service fixture for {len(models)} models to {output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
