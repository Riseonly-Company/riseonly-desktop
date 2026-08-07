#!/usr/bin/env python3
"""Re-derive FFmpeg's licence from the built artefact.

A single --enable-gpl relicenses this entire product under the GPL, and nothing
about the resulting binary announces it. scripts/build-ffmpeg-lgpl.sh passes the
right flags, but trusting the script means trusting that nobody edited it, that
the prefix was actually rebuilt after the edit, and that the prefix on this
machine is the one the script produced. So this reads the artefact instead.

Run against the vendored prefix, or with no argument to check every prefix
under vendor/ffmpeg/.
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
VENDOR = ROOT / "vendor" / "ffmpeg"

# Each of these, present in the configure line, makes the build GPL or worse.
FORBIDDEN_FLAGS = (
    "--enable-gpl",
    "--enable-nonfree",
)

# GPL-licensed libraries. Linking any of them has the same effect as the flag.
FORBIDDEN_LIBRARIES = (
    "libx264",
    "libx265",
    "libxvid",
    "libvidstab",
    "librubberband",
    "frei0r",
)

# LGPLv3 rather than LGPLv2.1. Not fatal, but it carries the anti-tivoisation
# and patent-retaliation terms, and adopting them should be a decision.
FLAGS_NEEDING_A_DECISION = ("--enable-version3",)


def configure_line(prefix: Path) -> str | None:
    info = prefix / "BUILD_INFO.txt"
    if info.exists():
        for line in info.read_text().splitlines():
            if line.startswith("configure="):
                return line.removeprefix("configure=")

    # No build record: fall back to the string libavutil bakes into itself.
    # This is the authoritative source — it is what avcodec_configuration()
    # returns at runtime — but it is only greppable, not parseable.
    for candidate in sorted(prefix.glob("lib/libavutil*.dylib")) + sorted(
        prefix.glob("lib/libavutil.so*")
    ) + sorted(prefix.glob("bin/avutil*.dll")):
        blob = candidate.read_bytes()
        match = re.search(rb"--prefix=[^\x00]{0,4000}", blob)
        if match:
            return match.group(0).decode("utf-8", "replace")

    return None


def check(prefix: Path) -> list[str]:
    line = configure_line(prefix)
    if line is None:
        return [f"{prefix}: no BUILD_INFO.txt and no configure string in the libraries"]

    problems = []

    for flag in FORBIDDEN_FLAGS:
        if flag in line:
            problems.append(f"{prefix}: {flag} relicenses the product under the GPL")

    for library in FORBIDDEN_LIBRARIES:
        if library in line:
            problems.append(f"{prefix}: links {library}, which is GPL")

    for flag in FLAGS_NEEDING_A_DECISION:
        if flag in line:
            problems.append(
                f"{prefix}: {flag} makes this LGPLv3, not LGPLv2.1 — a licensing decision"
            )

    # LGPL requires the user be able to substitute their own FFmpeg, which a
    # static link forecloses.
    if "--disable-shared" in line or "--enable-static" in line:
        problems.append(f"{prefix}: static linking removes the LGPL substitution right")

    return problems


def main() -> int:
    prefixes = (
        [Path(argument) for argument in sys.argv[1:]]
        if len(sys.argv) > 1
        else sorted(p for p in VENDOR.glob("*-*") if p.is_dir())
    )

    if not prefixes:
        print("ffmpeg-license: no prefix built; nothing to check")
        print("  build one with ./scripts/build-ffmpeg-lgpl.sh")
        return 0

    problems = []
    for prefix in prefixes:
        problems.extend(check(prefix))

    if problems:
        print(f"ffmpeg-license: {len(problems)} problem(s)\n")
        for problem in problems:
            print(f"  {problem}")
        return 1

    for prefix in prefixes:
        print(f"ffmpeg-license: {prefix.name} is LGPL-2.1, dynamically linked")
    return 0


if __name__ == "__main__":
    sys.exit(main())
