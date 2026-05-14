#!/usr/bin/env python3
"""Compile Markdown app compatibility entries into web/app_db.json and Rust metadata."""

from __future__ import annotations

import argparse
import datetime as dt
import json
import re
import sys
from pathlib import Path
from typing import Any

try:
    import yaml
except ImportError:  # pragma: no cover - exercised only on missing dependency.
    print(
        "error: PyYAML is required; install the python3-yaml package or pyyaml",
        file=sys.stderr,
    )
    sys.exit(2)


ENTRY_SCHEMA = "wemu.appcompat.v1"
DB_SCHEMA = "wemu.app_db.v1"

STATUS_VALUES = {
    "active-target",
    "ui-target",
    "regression",
    "boots",
    "playable",
    "broken",
    "unknown",
}
EXECUTABLE_ROLES = {"primary", "launcher", "setup", "helper", "test"}
ASSET_TYPES = {"directory", "file"}
ASSET_LOCATORS = {"named-directory", "named-file"}
MOUNT_DEVICES = {"fixed", "cdrom", "virtual"}

ID_RE = re.compile(r"^[a-z0-9][a-z0-9_-]*$")
FRONT_MATTER_RE = re.compile(r"\A---[ \t]*\r?\n(.*?)\r?\n---[ \t]*(?:\r?\n|\Z)", re.S)


class AppDbError(Exception):
    pass


def fail(path: Path, message: str) -> None:
    raise AppDbError(f"{path}: {message}")


def repo_root() -> Path:
    return Path(__file__).resolve().parents[1]


def relative_posix(path: Path, base: Path) -> str:
    try:
        return path.resolve().relative_to(base.resolve()).as_posix()
    except ValueError:
        return path.as_posix()


def normalize_yaml(value: Any) -> Any:
    if isinstance(value, dt.date):
        return value.isoformat()
    if isinstance(value, dict):
        return {str(key): normalize_yaml(child) for key, child in value.items()}
    if isinstance(value, list):
        return [normalize_yaml(child) for child in value]
    return value


def expect_dict(path: Path, data: dict[str, Any], key: str) -> dict[str, Any]:
    value = data.get(key)
    if not isinstance(value, dict):
        fail(path, f"`{key}` must be an object")
    return value


def expect_list(path: Path, data: dict[str, Any], key: str) -> list[Any]:
    value = data.get(key)
    if not isinstance(value, list):
        fail(path, f"`{key}` must be a list")
    return value


def expect_str(path: Path, data: dict[str, Any], key: str) -> str:
    value = data.get(key)
    if not isinstance(value, str) or not value:
        fail(path, f"`{key}` must be a non-empty string")
    return value


def expect_bool(path: Path, data: dict[str, Any], key: str) -> bool:
    value = data.get(key)
    if not isinstance(value, bool):
        fail(path, f"`{key}` must be a boolean")
    return value


def check_keys(path: Path, label: str, data: dict[str, Any], allowed: set[str]) -> None:
    extra = sorted(set(data) - allowed)
    if extra:
        fail(path, f"{label} has unknown field(s): {', '.join(extra)}")


def check_id(path: Path, label: str, value: str) -> None:
    if not ID_RE.fullmatch(value):
        fail(path, f"{label} must match {ID_RE.pattern}")


def check_iso_date(path: Path, key: str, value: str | None) -> None:
    if value is None:
        return
    if not isinstance(value, str):
        fail(path, f"`{key}` must be YYYY-MM-DD or null")
    try:
        dt.date.fromisoformat(value)
    except ValueError:
        fail(path, f"`{key}` must be YYYY-MM-DD")


def check_basename(path: Path, key: str, value: str) -> None:
    if "/" in value or "\\" in value:
        fail(path, f"`{key}` must be a basename, not a path")


def extract_front_matter(path: Path) -> tuple[dict[str, Any] | None, str]:
    text = path.read_text(encoding="utf-8")
    match = FRONT_MATTER_RE.match(text)
    if not match:
        return None, text
    raw_data = yaml.safe_load(match.group(1))
    if raw_data is None:
        raw_data = {}
    if not isinstance(raw_data, dict):
        fail(path, "front matter must be a YAML object")
    return normalize_yaml(raw_data), text[match.end() :]


def extract_summary(markdown: str) -> str:
    lines = markdown.splitlines()
    in_summary = False
    collected: list[str] = []
    for line in lines:
        if re.fullmatch(r"##\s+Summary\s*", line, flags=re.I):
            in_summary = True
            continue
        if not in_summary:
            continue
        if re.match(r"##\s+", line):
            break
        stripped = line.strip()
        if not stripped:
            if collected:
                break
            continue
        collected.append(stripped)
    return " ".join(collected)


def validate_executables(path: Path, data: dict[str, Any]) -> set[str]:
    executables = expect_list(path, data, "executables")
    if not executables:
        fail(path, "`executables` must contain at least one entry")

    ids: set[str] = set()
    for index, item in enumerate(executables):
        if not isinstance(item, dict):
            fail(path, f"`executables[{index}]` must be an object")
        check_keys(path, f"`executables[{index}]`", item, {"id", "filename", "role"})

        exe_id = expect_str(path, item, "id")
        check_id(path, f"`executables[{index}].id`", exe_id)
        if exe_id in ids:
            fail(path, f"duplicate executable id `{exe_id}`")
        ids.add(exe_id)

        filename = expect_str(path, item, "filename")
        check_basename(path, "filename", filename)

        role = expect_str(path, item, "role")
        if role not in EXECUTABLE_ROLES:
            fail(path, f"`role` must be one of: {', '.join(sorted(EXECUTABLE_ROLES))}")

    return ids


def validate_launch(path: Path, data: dict[str, Any], executable_ids: set[str]) -> None:
    launch = expect_dict(path, data, "launch")
    check_keys(path, "`launch`", launch, {"executable", "cwd"})

    executable = expect_str(path, launch, "executable")
    if executable not in executable_ids:
        fail(path, f"`launch.executable` references unknown executable `{executable}`")
    expect_str(path, launch, "cwd")


def validate_locale(path: Path, data: dict[str, Any]) -> None:
    locale = expect_dict(path, data, "locale")
    check_keys(path, "`locale`", locale, {"ansi_codepage"})
    expect_str(path, locale, "ansi_codepage")


def validate_assets(path: Path, data: dict[str, Any]) -> None:
    assets = expect_list(path, data, "required_assets")
    ids: set[str] = set()
    for index, item in enumerate(assets):
        if not isinstance(item, dict):
            fail(path, f"`required_assets[{index}]` must be an object")
        check_keys(
            path,
            f"`required_assets[{index}]`",
            item,
            {"id", "type", "name", "required", "purpose", "locator", "mount"},
        )

        asset_id = expect_str(path, item, "id")
        check_id(path, f"`required_assets[{index}].id`", asset_id)
        if asset_id in ids:
            fail(path, f"duplicate required asset id `{asset_id}`")
        ids.add(asset_id)

        asset_type = expect_str(path, item, "type")
        if asset_type not in ASSET_TYPES:
            fail(path, f"`type` must be one of: {', '.join(sorted(ASSET_TYPES))}")

        name = expect_str(path, item, "name")
        check_basename(path, "name", name)
        expect_bool(path, item, "required")
        expect_str(path, item, "purpose")

        locator = expect_str(path, item, "locator")
        if locator not in ASSET_LOCATORS:
            fail(path, f"`locator` must be one of: {', '.join(sorted(ASSET_LOCATORS))}")

        if "mount" in item:
            mount = expect_dict(path, item, "mount")
            check_keys(path, "`mount`", mount, {"drive", "device"})

            drive = expect_str(path, mount, "drive")
            if len(drive) != 1 or not drive.isascii() or not drive.isalpha() or drive != drive.upper():
                fail(path, "`mount.drive` must be one uppercase drive letter")

            device = expect_str(path, mount, "device")
            if device not in MOUNT_DEVICES:
                fail(path, f"`mount.device` must be one of: {', '.join(sorted(MOUNT_DEVICES))}")


def validate_verification(path: Path, data: dict[str, Any]) -> None:
    if "verification" not in data:
        return
    verification = expect_dict(path, data, "verification")
    check_keys(path, "`verification`", verification, {"last_checked", "method", "evidence"})

    check_iso_date(path, "verification.last_checked", verification.get("last_checked"))
    expect_str(path, verification, "method")
    evidence = expect_list(path, verification, "evidence")
    for index, item in enumerate(evidence):
        if not isinstance(item, str):
            fail(path, f"`verification.evidence[{index}]` must be a string")


def validate_entry(path: Path, data: dict[str, Any]) -> None:
    check_keys(
        path,
        "front matter",
        data,
        {
            "schema",
            "id",
            "name",
            "status",
            "updated",
            "executables",
            "launch",
            "locale",
            "required_assets",
            "verification",
        },
    )

    schema = expect_str(path, data, "schema")
    if schema != ENTRY_SCHEMA:
        fail(path, f"`schema` must be {ENTRY_SCHEMA}")

    app_id = expect_str(path, data, "id")
    check_id(path, "`id`", app_id)
    if path.stem != app_id:
        fail(path, f"`id` must match filename `{path.stem}`")

    expect_str(path, data, "name")
    status = expect_str(path, data, "status")
    if status not in STATUS_VALUES:
        fail(path, f"`status` must be one of: {', '.join(sorted(STATUS_VALUES))}")
    check_iso_date(path, "updated", expect_str(path, data, "updated"))

    executable_ids = validate_executables(path, data)
    validate_launch(path, data, executable_ids)
    validate_locale(path, data)
    validate_assets(path, data)
    validate_verification(path, data)


def load_entry(path: Path, base: Path) -> dict[str, Any] | None:
    if path.name.startswith("_"):
        return None

    data, body = extract_front_matter(path)
    if data is None:
        return None

    validate_entry(path, data)
    data["source"] = relative_posix(path, base)
    data["summary"] = extract_summary(body)
    return data


def compile_db(input_dir: Path, base: Path) -> dict[str, Any]:
    if not input_dir.is_dir():
        raise AppDbError(f"{input_dir}: input directory does not exist")

    apps: list[dict[str, Any]] = []
    for path in sorted(input_dir.glob("*.md")):
        entry = load_entry(path, base)
        if entry is not None:
            apps.append(entry)

    apps.sort(key=lambda app: app["id"])
    return {
        "schema": DB_SCHEMA,
        "entry_schema": ENTRY_SCHEMA,
        "apps": apps,
    }


def render_json(data: dict[str, Any]) -> str:
    return json.dumps(data, ensure_ascii=False, indent=2) + "\n"


def rust_str(value: str) -> str:
    return json.dumps(value, ensure_ascii=True)


def render_rust(data: dict[str, Any]) -> str:
    lines = [
        "// @generated by tools/compile_app_db.py; do not edit.",
        "",
        "pub(crate) static APP_DB: &[AppDbEntry] = &[",
    ]
    for app in data["apps"]:
        lines.extend(
            [
                "    AppDbEntry {",
                "        executables: &[",
            ]
        )
        for exe in app["executables"]:
            lines.extend(
                [
                    "            AppDbExecutable {",
                    f"                filename: {rust_str(exe['filename'])},",
                    "            },",
                ]
            )
        lines.extend(
            [
                "        ],",
                "        required_assets: &[",
            ]
        )
        for asset in app["required_assets"]:
            mount = asset.get("mount")
            if mount:
                mount_text = (
                    "Some(AppDbMount { "
                    f"drive: '{mount['drive']}', "
                    f"device: {rust_str(mount['device'])} "
                    "})"
                )
            else:
                mount_text = "None"
            lines.extend(
                [
                    "            AppDbRequiredAsset {",
                    f"                name: {rust_str(asset['name'])},",
                    f"                asset_type: {rust_str(asset['type'])},",
                    f"                locator: {rust_str(asset['locator'])},",
                    f"                mount: {mount_text},",
                    "            },",
                ]
            )
        lines.extend(
            [
                "        ],",
                "    },",
            ]
        )
    lines.extend(["];", ""])
    return "\n".join(lines)


def main(argv: list[str]) -> int:
    root = repo_root()
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path, default=root / "app_db")
    parser.add_argument("--output", type=Path, default=root / "web" / "app_db.json")
    parser.add_argument(
        "--rust-output",
        type=Path,
        default=root / "src" / "app_db_generated.rs",
        help="write compact Rust app DB metadata for runtime mount matching",
    )
    parser.add_argument(
        "--check",
        action="store_true",
        help="validate and fail if generated JSON/Rust differ from their outputs",
    )
    args = parser.parse_args(argv)

    try:
        data = compile_db(args.input, root)
        text = render_json(data)
        rust_text = render_rust(data)
        if args.check:
            if not args.output.exists() or args.output.read_text(encoding="utf-8") != text:
                print(f"{args.output}: app DB JSON is out of date", file=sys.stderr)
                return 1
            if (
                not args.rust_output.exists()
                or args.rust_output.read_text(encoding="utf-8") != rust_text
            ):
                print(f"{args.rust_output}: app DB Rust metadata is out of date", file=sys.stderr)
                return 1
            print(f"{args.output}: ok")
            print(f"{args.rust_output}: ok")
            return 0

        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(text, encoding="utf-8")
        args.rust_output.parent.mkdir(parents=True, exist_ok=True)
        args.rust_output.write_text(rust_text, encoding="utf-8")
        print(f"wrote {args.output} ({len(data['apps'])} app entries)")
        print(f"wrote {args.rust_output} ({len(data['apps'])} app entries)")
        return 0
    except AppDbError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
