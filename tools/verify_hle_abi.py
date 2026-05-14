#!/usr/bin/env python3
"""Verify simple HLE stdcall/cdecl cleanup against local MinGW headers.

This is intentionally a string-processing verifier. It does not run a C
preprocessor; it scans MinGW headers for straightforward function prototypes,
then compares their expected 32-bit stack cleanup with Rust HLE callbacks whose
cleanup can be read as a fixed HleResult::Retn(N).
"""

from __future__ import annotations

import argparse
import dataclasses
import re
import sys
from pathlib import Path


CALLTYPE_STDCALL = {
    "WINAPI",
    "APIENTRY",
    "CALLBACK",
    "PASCAL",
    "NTAPI",
    "SHSTDAPI",
    "SHSTDAPI_",
    "SHDOCAPI",
    "SHDOCAPI_",
    "STDAPI",
    "STDAPI_",
}
CALLTYPE_CDECL = {"WINAPIV", "__cdecl", "_cdecl", "cdecl", "CDECL", "__CRTDECL"}
CALLTYPE_RE = "|".join(
    re.escape(calltype) for calltype in sorted(CALLTYPE_STDCALL | CALLTYPE_CDECL, key=len, reverse=True)
)
POST_CALLTYPE_ATTR_RE = r"(?:\s+__[A-Z][A-Z0-9_]*)*"
TRAILING_ATTR_RE = r"(?:\s+__[A-Z][A-Z0-9_]*(?:\s*\([^;{}]*\))?)*"
TRAILING_DECL_ATTR_RE = re.compile(r"\s+__[A-Z][A-Z0-9_]*(?:\s*\([^()]*\))?\s*$")
IMPLICIT_CDECL_NAMES = {"__setusermatherr", "swprintf"}
IMPLICIT_CDECL_RE = "|".join(re.escape(name) for name in sorted(IMPLICIT_CDECL_NAMES))
SYNTHETIC_COM_PREFIXES = ("DDraw_", "DDS_", "DDP_", "DDC_", "DS_", "DSB_")
CRT_INTERNAL_OR_DATA = {
    "__dllonexit",
    "__getmainargs",
    "__initenv",
    "__lconv_init",
    "__p__commode",
    "__set_app_type",
    "__wgetmainargs",
    "__winitenv",
    "_amsg_exit",
    "_cexit",
    "_fpreset",
    "_initterm",
    "_iob",
    "_lock",
    "_unlock",
    "_wcmdln",
}
BY_VALUE_ARG_BYTES = {
    "POINT": 8,
    "POINTL": 8,
    "POINTS": 4,
    "SIZE": 8,
    "SIZEL": 8,
    "RECT": 16,
    "RECTL": 16,
}
KNOWN_HELPER_RETN = {
    "hle_ret_ok_0": 0,
    "hle_ret_ok_4": 4,
    "hle_ret_ok_8": 8,
    "hle_ret_ok_12": 12,
    "hle_ret_ok_16": 16,
    "hle_ret_ok_20": 20,
    "hle_ret_ok_24": 24,
    "hle_ret_ok_36": 36,
    "hle_ret_notimpl_8": 8,
    "hle_ret_notimpl_12": 12,
}
EXPECTED_RETN_OVERRIDES = {
    # MinGW exposes these as cdecl compiler intrinsics in winbase.h, but old
    # PE imports call the KERNEL32 exports as stdcall entry points.
    "InterlockedDecrement": 4,
    "InterlockedExchange": 8,
    "InterlockedIncrement": 4,
}


@dataclasses.dataclass(frozen=True)
class Prototype:
    name: str
    calltype: str
    args: tuple[str, ...]
    header: Path
    line: int

    @property
    def expected_retn(self) -> int:
        if self.calltype in CALLTYPE_CDECL:
            return 0
        return sum(arg_stack_bytes(arg) for arg in self.args)


@dataclasses.dataclass(frozen=True)
class Mapping:
    api: str
    callback: str
    source: Path
    line: int


def strip_comments(text: str) -> str:
    def keep_newlines(match: re.Match[str]) -> str:
        return " " + ("\n" * match.group(0).count("\n"))

    text = re.sub(r"/\*.*?\*/", keep_newlines, text, flags=re.S)
    text = re.sub(r"//.*", " ", text)
    return text.replace("\\\n", " ")


def split_args(arg_text: str) -> tuple[str, ...]:
    arg_text = " ".join(arg_text.strip().split())
    if not arg_text or arg_text.lower() == "void":
        return ()
    args: list[str] = []
    depth = 0
    start = 0
    for i, ch in enumerate(arg_text):
        if ch == "(":
            depth += 1
        elif ch == ")":
            depth = max(0, depth - 1)
        elif ch == "," and depth == 0:
            args.append(arg_text[start:i].strip())
            start = i + 1
    args.append(arg_text[start:].strip())
    return tuple(arg for arg in args if arg and arg != "void")


def arg_stack_bytes(arg: str) -> int:
    arg = " ".join(arg.replace("__MINGW_ATTRIB_NONNULL", " ").split())
    if arg == "...":
        return 0
    if "*" in arg or "(WINAPI *" in arg or "(CALLBACK *" in arg:
        return 4
    wide = (
        "__int64",
        "LONGLONG",
        "ULONGLONG",
        "DWORDLONG",
        "QWORD",
        "double",
    )
    if any(token in arg for token in wide):
        return 8
    tokens = re.findall(r"[A-Za-z_][A-Za-z0-9_]*", arg)
    for token in reversed(tokens):
        if token in BY_VALUE_ARG_BYTES:
            return BY_VALUE_ARG_BYTES[token]
    return 4


def statement_line(text: str, offset: int) -> int:
    return text.count("\n", 0, offset) + 1


def strip_trailing_decl_attrs(statement: str) -> str:
    while True:
        stripped = TRAILING_DECL_ATTR_RE.sub("", statement)
        if stripped == statement:
            return statement
        statement = stripped


def parse_headers(include_dir: Path) -> dict[str, Prototype]:
    prototypes: dict[str, Prototype] = {}
    if not include_dir.is_dir():
        return prototypes

    proto_re = re.compile(
        rf"\b(?P<calltype>{CALLTYPE_RE})\b{POST_CALLTYPE_ATTR_RE}\s+"
        r"(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*\((?P<args>.*)\)"
        rf"\s*{TRAILING_ATTR_RE}\s*$",
        re.S,
    )
    macro_proto_re = re.compile(
        r"\b(?P<calltype>SHSTDAPI_|SHDOCAPI_|STDAPI_)\s*\([^)]*\)\s+"
        r"(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*\((?P<args>.*)\)"
        rf"\s*{TRAILING_ATTR_RE}\s*$",
        re.S,
    )
    bare_macro_proto_re = re.compile(
        r"\b(?P<calltype>SHSTDAPI|SHDOCAPI|STDAPI)\s+"
        r"(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*\((?P<args>.*)\)"
        rf"\s*{TRAILING_ATTR_RE}\s*$",
        re.S,
    )
    implicit_proto_re = re.compile(
        rf"\b(?P<name>{IMPLICIT_CDECL_RE})\s*\((?P<args>.*)\)"
        rf"\s*{TRAILING_ATTR_RE}\s*$",
        re.S,
    )
    implicit_def_re = re.compile(
        rf"(^|\n)[^;\n{{}}]*\b(?P<name>{IMPLICIT_CDECL_RE})\s*"
        r"\((?P<args>[^{};]*)\)\s*\{",
        re.S,
    )
    headers = sorted(list(include_dir.glob("*.h")) + list(include_dir.glob("*.inl")))
    for header in headers:
        try:
            raw = header.read_text(errors="ignore")
        except OSError:
            continue
        text = strip_comments(raw)
        text = re.sub(r'extern\s+"C(?:\+\+)?"\s*\{', " ", text)
        start = 0
        for statement in text.split(";"):
            offset = start
            start += len(statement) + 1
            flat = " ".join(statement.split())
            if "(" not in flat:
                continue
            if "{" in flat or "}" in flat:
                suffix = flat[max(flat.rfind("{"), flat.rfind("}")) + 1 :].strip()
                if not re.search(rf"\b({CALLTYPE_RE})\b", suffix):
                    continue
                flat = suffix
            flat = strip_trailing_decl_attrs(flat)
            match = (
                macro_proto_re.search(flat)
                or bare_macro_proto_re.search(flat)
                or proto_re.search(flat)
                or implicit_proto_re.search(flat)
            )
            if not match:
                continue
            name = match.group("name")
            calltype = match.groupdict().get("calltype") or "__cdecl"
            args = split_args(match.group("args"))
            name_offset = offset + statement.find(name)
            line = statement_line(text, name_offset if name_offset >= offset else offset)
            prototypes.setdefault(
                name,
                Prototype(name, calltype, args, header, line),
            )
        for match in implicit_def_re.finditer(text):
            name = match.group("name")
            prototypes.setdefault(
                name,
                Prototype(
                    name,
                    "__cdecl",
                    split_args(match.group("args")),
                    header,
                    statement_line(text, match.start("name")),
                ),
            )
    return prototypes


def extract_balanced_block(text: str, marker: str) -> str:
    block, _ = extract_balanced_block_with_offset(text, marker)
    return block


def extract_balanced_block_with_offset(text: str, marker: str) -> tuple[str, int]:
    start = text.find(marker)
    if start < 0:
        return "", 0
    brace = text.find("{", start)
    if brace < 0:
        return "", 0
    depth = 0
    for i in range(brace, len(text)):
        if text[i] == "{":
            depth += 1
        elif text[i] == "}":
            depth -= 1
            if depth == 0:
                return text[brace + 1 : i], brace + 1
    return "", 0


def parse_callback_for(path: Path) -> list[Mapping]:
    text = path.read_text()
    block, block_offset = extract_balanced_block_with_offset(text, "fn callback_for")
    mappings: list[Mapping] = []
    arm_re = re.compile(
        r'(?P<patterns>"[^"]+"\s*(?:\|\s*"[^"]+"\s*)*)=>\s*(?:\{\s*)?'
        r"(?P<callback>hle_[A-Za-z0-9_]+)\b",
        re.S,
    )
    for match in arm_re.finditer(block):
        patterns = match.group("patterns")
        cb = match.group("callback")
        names = re.findall(r'"([^"]+)"', patterns)
        for name in names:
            if name.startswith("#"):
                continue
            local_offset = patterns.find(f'"{name}"')
            line = statement_line(
                text,
                block_offset + match.start("patterns") + max(local_offset, 0),
            )
            mappings.append(Mapping(name, cb, path, line))
    return mappings


def parse_tuple_registrations(path: Path) -> list[Mapping]:
    text = path.read_text()
    mappings: list[Mapping] = []
    tuple_re = re.compile(r'\("([^"]+)",\s*(hle_[A-Za-z0-9_]+)\s+as\s+HleCallback\)')
    for match in tuple_re.finditer(text):
        name = match.group(1)
        if name.startswith("#"):
            continue
        mappings.append(Mapping(name, match.group(2), path, statement_line(text, match.start())))
    return mappings


def parse_rust_mappings(src_dir: Path) -> list[Mapping]:
    mappings: list[Mapping] = []
    base = src_dir / "hle_base.rs"
    if base.exists():
        mappings.extend(parse_callback_for(base))
    for path in sorted(src_dir.glob("hle*.rs")):
        mappings.extend(parse_tuple_registrations(path))
    seen: set[tuple[str, str]] = set()
    unique: list[Mapping] = []
    for mapping in mappings:
        key = (mapping.api, mapping.callback)
        if key not in seen:
            seen.add(key)
            unique.append(mapping)
    return unique


def prototype_signature(proto: Prototype) -> str:
    args = ", ".join(proto.args) if proto.args else "void"
    return f"{proto.calltype} {proto.name}({args})"


def skip_category(name: str) -> str:
    if name.startswith(SYNTHETIC_COM_PREFIXES):
        return "synthetic_com"
    if name in CRT_INTERNAL_OR_DATA:
        return "crt_internal_or_data"
    return "unknown"


def extract_function_body(text: str, fn_name: str) -> str:
    match = re.search(rf"\bfn\s+{re.escape(fn_name)}\b", text)
    if not match:
        return ""
    brace = text.find("{", match.end())
    if brace < 0:
        return ""
    depth = 0
    for i in range(brace, len(text)):
        if text[i] == "{":
            depth += 1
        elif text[i] == "}":
            depth -= 1
            if depth == 0:
                return text[brace + 1 : i]
    return ""


def parse_callback_retns(src_dir: Path) -> dict[str, tuple[int, ...]]:
    by_callback: dict[str, set[int]] = {
        name: {value} for name, value in KNOWN_HELPER_RETN.items()
    }
    for path in sorted(src_dir.glob("hle*.rs")):
        text = path.read_text()
        for match in re.finditer(r"\bfn\s+(hle_[A-Za-z0-9_]+)\b", text):
            name = match.group(1)
            body = extract_function_body(text, name)
            values = {
                int(value, 0)
                for value in re.findall(r"HleResult::Retn\((0x[0-9a-fA-F]+|\d+)\)", body)
            }
            if values:
                by_callback.setdefault(name, set()).update(values)
    return {name: tuple(sorted(values)) for name, values in by_callback.items()}


def default_include_dir() -> Path:
    candidates = [
        Path("/usr/share/mingw-w64/include"),
        Path("/usr/i686-w64-mingw32/include"),
        Path("/usr/x86_64-w64-mingw32/include"),
    ]
    for candidate in candidates:
        if (candidate / "windows.h").exists():
            return candidate
    return candidates[0]


def resolve_name(name: str, choices: set[str]) -> str | None:
    if name in choices:
        return name
    folded = name.lower()
    for choice in sorted(choices):
        if choice.lower() == folded:
            return choice
    return None


def print_query(
    names: list[str],
    prototypes: dict[str, Prototype],
    mappings: list[Mapping],
    retns: dict[str, tuple[int, ...]],
) -> int:
    mappings_by_api: dict[str, list[Mapping]] = {}
    for mapping in mappings:
        mappings_by_api.setdefault(mapping.api, []).append(mapping)

    failed = False
    known_names = set(prototypes) | set(mappings_by_api)
    for raw_name in names:
        name = resolve_name(raw_name, known_names)
        if name is None:
            print(f"{raw_name}: not found in MinGW prototypes or Rust HLE mappings")
            failed = True
            continue

        print(f"{raw_name}:")
        proto = prototypes.get(name)
        if proto is None:
            print("  header: not found")
        else:
            expected = EXPECTED_RETN_OVERRIDES.get(name, proto.expected_retn)
            suffix = " (override)" if expected != proto.expected_retn else ""
            print(
                f"  header: {prototype_signature(proto)}; "
                f"expected Retn({expected}){suffix}; {proto.header}:{proto.line}"
            )

        api_mappings = mappings_by_api.get(name, [])
        if not api_mappings:
            print("  rust: not mapped")
        for mapping in api_mappings:
            values = retns.get(mapping.callback, ())
            if len(values) == 1:
                actual = f"Retn({values[0]})"
            elif values:
                actual = "dynamic Retn{" + ", ".join(str(value) for value in values) + "}"
            else:
                actual = "Retn unknown"
            status = ""
            if proto is not None and len(values) == 1:
                expected = EXPECTED_RETN_OVERRIDES.get(name, proto.expected_retn)
                status = " ok" if values[0] == expected else " mismatch"
            print(
                f"  rust: {name} -> {mapping.callback} {actual}; "
                f"{mapping.source}:{mapping.line}{status}"
            )
    return 1 if failed else 0


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Check Rust HLE Retn byte counts against local MinGW headers."
    )
    parser.add_argument("--src", default="src", type=Path)
    parser.add_argument("--include-dir", default=default_include_dir(), type=Path)
    parser.add_argument(
        "--query",
        nargs="+",
        metavar="API",
        help="print MinGW prototype and Rust HLE cleanup info for one or more APIs",
    )
    parser.add_argument(
        "--list-skips",
        action="store_true",
        help="print APIs skipped because no matching MinGW header prototype was found",
    )
    parser.add_argument("--verbose", action="store_true")
    args = parser.parse_args()

    prototypes = parse_headers(args.include_dir)
    mappings = parse_rust_mappings(args.src)
    retns = parse_callback_retns(args.src)

    if args.query:
        return print_query(args.query, prototypes, mappings, retns)

    checked = 0
    skipped_no_proto = 0
    skipped_dynamic = 0
    skipped_by_category: dict[str, list[Mapping]] = {}
    mismatches: list[str] = []

    for mapping in mappings:
        proto = prototypes.get(mapping.api)
        if proto is None:
            skipped_no_proto += 1
            skipped_by_category.setdefault(skip_category(mapping.api), []).append(mapping)
            continue
        values = retns.get(mapping.callback, ())
        if len(values) != 1:
            skipped_dynamic += 1
            if args.verbose:
                print(
                    f"skip dynamic {mapping.api}->{mapping.callback}: Retn values {values or 'unknown'}"
                )
            continue
        actual = values[0]
        expected = EXPECTED_RETN_OVERRIDES.get(mapping.api, proto.expected_retn)
        checked += 1
        if actual != expected:
            mismatches.append(
                f"{mapping.api}: Rust {mapping.callback} Retn({actual}), "
                f"header expects Retn({expected}) from {proto.calltype} "
                f"{proto.header}:{proto.line} args={len(proto.args)}; "
                f"mapped at {mapping.source}:{mapping.line}"
            )

    if mismatches:
        print("HLE ABI mismatches:")
        for mismatch in mismatches:
            print(f"  {mismatch}")
    if args.list_skips:
        print("HLE ABI skipped_no_header:")
        for category in sorted(skipped_by_category):
            entries = sorted(skipped_by_category[category], key=lambda item: item.api)
            print(f"  {category} ({len(entries)}):")
            for mapping in entries:
                print(
                    f"    {mapping.api} -> {mapping.callback}; "
                    f"{mapping.source}:{mapping.line}"
                )
    print(
        "HLE ABI verifier: "
        f"checked={checked} mismatches={len(mismatches)} "
        f"skipped_no_header={skipped_no_proto} skipped_dynamic={skipped_dynamic} "
        f"headers={len(prototypes)} include_dir={args.include_dir}"
    )
    return 1 if mismatches else 0


if __name__ == "__main__":
    raise SystemExit(main())
