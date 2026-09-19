#!/usr/bin/env python3
"""Convert 'dw `XXXXXXXX' RGBDS graphics literals into 'db $lo,$hi' byte pairs.

Usage:
    gfx_to_db.py [input_file] [-o output_file]

With no input_file, reads from stdin. With no -o, writes to stdout.
"""

import argparse
import re
import sys

DW_GFX_RE = re.compile(
    r"^(?P<indent>[ \t]*)[Dd][Ww]\s+"
    r"(?P<values>`[0-3]{1,8}(?:\s*,\s*`[0-3]{1,8})*)"
    r"\s*(?P<comment>;.*)?$"
)
GFX_RE = re.compile(r"`([0-3]{1,8})")


def gfx_to_bytes(digits: str) -> tuple[int, int]:
    """Split a graphics literal's pixel digits into its (LSB, MSB) bit-plane bytes."""
    lower = upper = 0
    for ch in digits:
        pixel = int(ch)
        lower = (lower << 1) | (pixel & 1)
        upper = (upper << 1) | (pixel >> 1)
    return lower, upper


def convert_line(line: str) -> str:
    body, newline = (line[:-1], "\n") if line.endswith("\n") else (line, "")

    match = DW_GFX_RE.match(body)
    if not match:
        return line

    pairs = []
    for digits in GFX_RE.findall(match.group("values")):
        lower, upper = gfx_to_bytes(digits)
        pairs.append(f"${lower:02X},${upper:02X}")

    result = f"{match.group('indent')}db {','.join(pairs)}"
    if match.group("comment"):
        result += f" {match.group('comment')}"
    return result + newline


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", nargs="?", help="input file (default: stdin)")
    parser.add_argument("-o", "--output", help="output file (default: stdout)")
    args = parser.parse_args()

    in_f = open(args.input) if args.input else sys.stdin
    out_f = open(args.output, "w") if args.output else sys.stdout

    try:
        for line in in_f:
            out_f.write(convert_line(line))
    finally:
        if args.input:
            in_f.close()
        if args.output:
            out_f.close()


if __name__ == "__main__":
    main()
