#!/usr/bin/env python3
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
VENDOR = ROOT / "vendor" / "rlottie"

KNOWN_LICENCES = {
    "COPYING.FTL": "FreeType Project License (BSD-style)",
    "COPYING.MIT": "MIT",
    "COPYING.MPL": "Mozilla Public License 2.0 (file-level copyleft)",
    "COPYING.PIX": "pixman (MIT)",
    "COPYING.RPD": "rapidjson (MIT)",
    "COPYING.SKIA": "Skia (BSD-3-Clause)",
    "COPYING.STB": "stb (public domain / MIT)",
}

FORBIDDEN_TEXT = (
    "GNU General Public License",
    "GNU Affero General Public",
    "GNU Lesser General Public",
)

MPL_MARKER = "Mozilla Public"

COMPILED_SUFFIXES = {".c", ".cpp", ".cc", ".h", ".hpp", ".S"}


def sources(tree: Path) -> list[Path]:
    return sorted(
        path
        for directory in (tree / "src", tree / "inc")
        if directory.is_dir()
        for path in directory.rglob("*")
        if path.is_file() and path.suffix in COMPILED_SUFFIXES
    )


def check(tree: Path) -> tuple[list[str], list[Path]]:
    problems: list[str] = []
    mpl_files: list[Path] = []

    licences = tree / "licenses"
    if not licences.is_dir():
        problems.append(f"{tree}: no licenses/ directory; this is not an rlottie tree")
        return problems, mpl_files

    found = {path.name for path in licences.iterdir() if path.is_file()}
    for unknown in sorted(found - set(KNOWN_LICENCES)):
        problems.append(
            f"{tree}: licenses/{unknown} is a licence nobody has assessed"
        )

    files = sources(tree)
    if not files:
        problems.append(f"{tree}: no compilable sources found")
        return problems, mpl_files

    for path in files:
        text = path.read_text(encoding="utf-8", errors="replace")
        for forbidden in FORBIDDEN_TEXT:
            if forbidden in text:
                problems.append(
                    f"{path.relative_to(tree)}: contains '{forbidden}'"
                )
        if MPL_MARKER in text:
            mpl_files.append(path.relative_to(tree))

    return problems, mpl_files


def main() -> int:
    trees = (
        [Path(argument) for argument in sys.argv[1:]]
        if len(sys.argv) > 1
        else sorted(p for p in VENDOR.glob("rlottie-*") if p.is_dir())
    )

    if not trees:
        print("rlottie-license: nothing vendored; nothing to check")
        print("  vendor it with ./scripts/vendor-rlottie.sh")
        return 0

    problems: list[str] = []
    for tree in trees:
        tree_problems, mpl_files = check(tree)
        problems.extend(tree_problems)
        if not tree_problems:
            print(f"rlottie-license: {tree.name} carries no copyleft product obligation")
            for path in mpl_files:
                print(f"  MPL-2.0, source must ship with the release: {path}")

    if problems:
        print(f"\nrlottie-license: {len(problems)} problem(s)\n")
        for problem in problems:
            print(f"  {problem}")
        return 1

    return 0


if __name__ == "__main__":
    sys.exit(main())
