#!/usr/bin/env python3
"""
Convert a Tera Term Macro (.ttl) flashing configuration into ipl.json.

Usage:
    python3 ttl_to_ipl.py Flash_Bootloader_V4M.ttl
    python3 ttl_to_ipl.py Flash_Bootloader_V4M.ttl -o ipl.json

The script reads IMGNM, SAVEADR, IMGADR, WRITESEL, and FLASHWRITER_FILE.
It preserves address values as strings (including leading zeros).
"""
from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path


ASSIGNMENT_RE = re.compile(
    r"""^\s*(?P<var>[A-Za-z_]\w*)\s*=\s*(?P<value>'[^']*'|"[^"]*"|[^\s;]+)\s*(?:;.*)?$"""
)
ARRAY_ASSIGNMENT_RE = re.compile(
    r"""^\s*(?P<var>[A-Za-z_]\w*)\s*\[\s*(?P<index>\d+)\s*\]\s*=\s*(?P<value>'[^']*'|"[^"]*"|[^\s;]+)\s*(?:;.*)?$"""
)


def unquote(value: str) -> str:
    value = value.strip()
    if len(value) >= 2 and value[0] == value[-1] and value[0] in ("'", '"'):
        return value[1:-1]
    return value


def parse_ttl(path: Path) -> dict:
    scalars: dict[str, str] = {}
    arrays: dict[str, dict[int, str]] = {}

    for line_number, raw_line in enumerate(
        path.read_text(encoding="utf-8-sig").splitlines(), start=1
    ):
        line = raw_line.strip()
        if not line or line.startswith(";"):
            continue

        match = ARRAY_ASSIGNMENT_RE.match(raw_line)
        if match:
            name = match.group("var").upper()
            index = int(match.group("index"))
            arrays.setdefault(name, {})[index] = unquote(match.group("value"))
            continue

        match = ASSIGNMENT_RE.match(raw_line)
        if match:
            name = match.group("var").upper()
            value = unquote(match.group("value"))
            # Ignore unrelated Tera Term assignments, but retain configuration scalars.
            if name in {"ARRAY_NUM", "FLASHWRITER_FILE"}:
                scalars[name] = value

    if "FLASHWRITER_FILE" not in scalars:
        raise ValueError("Could not find FLASHWRITER_FILE = '...' in TTL file.")

    try:
        count = int(scalars["ARRAY_NUM"])
    except (KeyError, ValueError) as exc:
        raise ValueError("Could not find a valid integer ARRAY_NUM in TTL file.") from exc

    required = ("IMGNM", "SAVEADR", "IMGADR", "WRITESEL")
    for name in required:
        if name not in arrays:
            raise ValueError(f"Could not find any {name}[index] assignments.")

    images = []
    for index in range(count):
        missing = [
            name for name in required
            if index not in arrays[name]
        ]
        if missing:
            raise ValueError(
                f"Image index {index} is missing assignment(s): {', '.join(missing)}."
            )

        writer = arrays["WRITESEL"][index].strip().lower()
        if writer == "flash":
            emmc = False
        elif writer == "emmc":
            emmc = True
        else:
            raise ValueError(
                f"Unsupported WRITESEL[{index}] value {arrays['WRITESEL'][index]!r}; "
                "expected 'Flash' or 'eMMC'."
            )

        images.append({
            "name": arrays["IMGNM"][index],
            "save": arrays["SAVEADR"][index],
            "address": arrays["IMGADR"][index],
            "emmc": emmc,
        })

    # Flag array entries outside ARRAY_NUM rather than silently dropping them.
    for name in required:
        extra = sorted(index for index in arrays[name] if index >= count)
        if extra:
            print(
                f"Warning: {name} has index/indices beyond ARRAY_NUM: {extra}",
                file=sys.stderr,
            )

    return {
        "flashWriter": scalars["FLASHWRITER_FILE"],
        "images": images,
    }


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Convert a Tera Term Macro (.ttl) file to ipl.json."
    )
    parser.add_argument("ttl_file", type=Path, help="Input .ttl file")
    parser.add_argument(
        "-o", "--output", type=Path,
        help="Output JSON path (default: same basename with .json extension)",
    )
    args = parser.parse_args()

    if not args.ttl_file.is_file():
        parser.error(f"Input file does not exist: {args.ttl_file}")

    output = args.output or args.ttl_file.with_suffix(".json")

    try:
        result = parse_ttl(args.ttl_file)
        output.write_text(
            json.dumps(result, indent=2, ensure_ascii=False) + "\n",
            encoding="utf-8",
        )
    except (OSError, ValueError) as exc:
        print(f"Error: {exc}", file=sys.stderr)
        return 1

    print(f"Converted {args.ttl_file} -> {output} ({len(result['images'])} images)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
