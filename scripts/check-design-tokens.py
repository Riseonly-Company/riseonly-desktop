#!/usr/bin/env python3
"""Keep colour and length literals inside rise-theme, where the design lives.

PHASES.txt Phase 5 gates on "no literal colour or length outside rise-theme".
A component that hard-codes #1E1E1E survives exactly one appearance and a
component that hard-codes px(45.0) survives exactly one density multiplier, so
both defects are invisible until the storybook is rendered light, dark and at
three densities — which is late. This makes the gate mechanical instead.

  1. A colour literal is rgb()/rgba()/hsla(), an Rgba/Hsla struct literal, one
     of gpui's named colours (white, black, red, transparent_black, ...) or a
     bare "#RRGGBB" string. All of them belong in tokens/palette.rs.
  2. A length literal is px(45.0), rems(1.5) or Pixels(...) built from a number.
     px(theme.button.height_300) is fine: the number came from the theme.
  3. A baked scale step is a tailwind-ish gpui helper that carries the number in
     its name — .h_10() is rems(2.5), .rounded_lg() is rems(0.5), .text_sm() is
     0.875rem. Tailwind's scale is not our scale.

WHAT THIS DOES NOT SEE, and would need a compiler to see

  A regex reads one line at a time and cannot follow a value. px(SIDEBAR_WIDTH)
  where the constant is 320.0 two files away reads as clean here, and so does a
  colour assembled arithmetically or loaded from a string built at runtime. The
  check raises the cost of hard-coding a number carelessly; it does not prove
  the absence of one. Nor does it know whether a number is a design decision at
  all — hence the escape hatch below, which is why the escape count is printed
  on every run rather than hidden.

  Brace tracking, used to find #[cfg(test)] blocks, ignores raw strings and
  multi-line string literals. A `{` inside one shifts the block boundary. The
  tree is rustfmt-clean (cargo make lint runs fmt-check first), so this has not
  bitten yet, but it is a real hole.

WHAT IT DELIBERATELY LEAVES ALONE

  crates/rise-theme        the numbers belong there; it is not in scope at all
  #[cfg(test)] and tests/  a test asserting px(45.0) is the reference height is
                           the point of the test
  STORYBOOK_PATHS          the storybook lays components out and needs its own
                           gutters. An explicit path list, never a name match:
                           "anything called storybook" is how a second, quieter
                           design system gets started.
  `// design-token-exempt: <reason>`
                           one line, one reason, mandatory. Reasonless escapes
                           are themselves violations, and more than
                           ESCAPE_CEILING of them fails the build.
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

SCOPE = (
    "crates/rise-ui",
    "crates/rise-widgets",
    "crates/rise-navigation",
    "app",
)

# Directory prefixes, relative to ROOT. Every entry is a deliberate decision
# with a reason, not a convenience: adding one exempts a whole subtree.
STORYBOOK_PATHS = (
    # The Phase 5 gate's own surface: it arranges components on a page, so its
    # gutters, columns and swatch sizes are layout of the harness, not tokens.
    "crates/rise-ui/src/storybook",
    "app/src/app/storybook",
)

ESCAPE_CEILING = 12
ESCAPE = re.compile(r"//\s*design-token-exempt\b\s*:?\s*(.*)$")

_COLOUR_FNS = (
    "transparent_black|transparent_white|opaque_grey|rgba|rgb|hsla"
    "|white|black|red|blue|green|yellow"
)
# Anchored away from a leading dot on purpose: gpui's named colours are free
# functions, and self.green() or palette.rgba() is somebody else's method.
COLOUR_CALL = re.compile(rf"(?:^|[^.\w])(?:gpui::)?({_COLOUR_FNS})\s*\(")
COLOUR_STRUCT = re.compile(r"\b(Rgba|Hsla)\s*\{")
# `fn background(..) -> Hsla {` is a signature, not a literal, and so are the
# impl and generic forms. Without this every accessor that returns a colour is
# reported as inventing one.
TYPE_POSITION = re.compile(r"(->|:|<|\bas|\bstruct|\benum|\bimpl|\bfor|\btype)\s*$")
COLOUR_HEX = re.compile(r"\"#[0-9a-fA-F]{3,8}\"")

# A numeric first character in the argument is the whole test: px(theme.radius)
# reads the token, px(10.0) invents one. Zero is not a design decision.
LENGTH_CALL = re.compile(r"(?:^|[^.\w])(?:gpui::)?(px|rems|Pixels|DevicePixels)\s*\(\s*(-?[\d.]+)")

_BOX_PREFIXES = (
    "min_size|max_size|min_w|min_h|max_w|max_h|gap_x|gap_y|inset|bottom|right"
    "|left|size|gap|top|mt|mb|my|mx|ml|mr|pt|pb|py|px|pl|pr|w|h|m|p"
)
_CORNER_PREFIXES = "rounded_tl|rounded_tr|rounded_bl|rounded_br|rounded_t|rounded_b|rounded_l|rounded_r|rounded"
_BORDER_PREFIXES = "border_t|border_b|border_l|border_r|border_x|border_y|border"
_NAMED_STEPS = "3xl|2xl|xs|sm|md|lg|xl|base"

# .w_10() and .p_4() bake a rem step. Fractions (.w_1_2()), .w_full() and
# .w_auto() are relative, not lengths, so they never match: the suffix must be a
# single integer group followed immediately by (). Zero is filtered at the call.
BOX_STEP = re.compile(rf"\.(?:{_BOX_PREFIXES})_(?:neg_)?(\d+|px)\s*\(\s*\)")
CORNER_STEP = re.compile(rf"\.(?:{_CORNER_PREFIXES})_({_NAMED_STEPS})\s*\(\s*\)")
TEXT_STEP = re.compile(rf"\.text_({_NAMED_STEPS})\s*\(\s*\)")
# border_1 is the hairline — there is exactly one of it in any design system and
# gpui offers no other way to draw it. border_2 and up are choices.
BORDER_STEP = re.compile(rf"\.(?:{_BORDER_PREFIXES})_(\d+)\s*\(\s*\)")

CFG_TEST = re.compile(r"#\[cfg\((?:[^)]*\b)?test\b")
CHAR_LITERAL = re.compile(r"'(?:\\.|[^\\'])'")


class Violations:
    def __init__(self) -> None:
        self.items: list[str] = []
        self.escapes: list[str] = []

    def add(self, path: Path, lineno: int, rule: str, detail: str) -> None:
        rel = path.relative_to(ROOT).as_posix()
        self.items.append(f"{rel}:{lineno}  [{rule}] {detail}")

    def escape(self, path: Path, lineno: int, reason: str) -> None:
        rel = path.relative_to(ROOT).as_posix()
        self.escapes.append(f"{rel}:{lineno}  {reason}")

    def report(self) -> int:
        over_ceiling = len(self.escapes) > ESCAPE_CEILING
        if not self.items and not self.escapes:
            print("design-tokens: clean")
            return 0

        if self.items:
            print(f"design-tokens: {len(self.items)} violation(s)\n")
            for item in self.items:
                print(f"  {item}")
        else:
            print("design-tokens: clean")

        if self.escapes:
            print(f"\nescape hatches: {len(self.escapes)} of {ESCAPE_CEILING} allowed")
            for item in self.escapes:
                print(f"  {item}")
        if over_ceiling:
            print(
                f"\nthe escape hatch is meant to be rare and {len(self.escapes)} "
                f"is past the ceiling of {ESCAPE_CEILING}: move these into "
                "crates/rise-theme, or argue the ceiling up in scripts/CONTEXT.txt."
            )

        if self.items or over_ceiling:
            print("\nSee PHASES.txt Phase 5 and scripts/CONTEXT.txt for the rule.")
            return 1
        return 0


def strip_comments(line: str) -> str:
    return line.split("//", 1)[0]


def code_only(line: str) -> str:
    line = CHAR_LITERAL.sub("' '", line)
    out: list[str] = []
    i = 0
    n = len(line)
    in_string = False
    while i < n:
        ch = line[i]
        if in_string:
            if ch == "\\":
                i += 2
                continue
            if ch == '"':
                in_string = False
            i += 1
            continue
        if ch == '"':
            in_string = True
            i += 1
            continue
        if ch == "/" and i + 1 < n and line[i + 1] == "/":
            break
        out.append(ch)
        i += 1
    return "".join(out)


def test_lines(lines: list[str]) -> set[int]:
    inside: set[int] = set()
    depth = 0
    floor = 0
    pending = False
    skipping = False

    for lineno, raw in enumerate(lines, 1):
        code = code_only(raw)
        opens = code.count("{")
        closes = code.count("}")

        if not skipping and CFG_TEST.search(code):
            pending = True

        if pending and opens:
            skipping = True
            floor = depth
            pending = False
        elif pending and ";" in code:
            pending = False

        if skipping:
            inside.add(lineno)

        depth += opens - closes

        if skipping and depth <= floor:
            skipping = False

    return inside


def is_exempt(rel: str) -> bool:
    if any(part == "tests" for part in rel.split("/")) or rel.endswith("/tests.rs"):
        return True
    return any(rel == p or rel.startswith(p + "/") for p in STORYBOOK_PATHS)


def sources() -> list[Path]:
    out: list[Path] = []
    for area in SCOPE:
        src = ROOT / area / "src"
        if src.is_dir():
            out.extend(sorted(src.rglob("*.rs")))
    return out


def scan(path: Path, violations: Violations) -> None:
    try:
        lines = path.read_text().splitlines()
    except UnicodeDecodeError:
        return

    skip = test_lines(lines)

    for lineno, raw in enumerate(lines, 1):
        if lineno in skip:
            continue

        escape = ESCAPE.search(raw)
        if escape:
            reason = escape.group(1).strip()
            if reason:
                violations.escape(path, lineno, reason)
                continue
            violations.add(path, lineno, "escape-unexplained",
                           "design-token-exempt needs a reason after the colon")
            continue

        line = code_only(raw)

        match = COLOUR_CALL.search(line)
        struct = COLOUR_STRUCT.search(line)
        if match:
            violations.add(path, lineno, "colour-literal",
                           f"{match.group(1)}() names a colour rise-theme should own")
        elif struct and not TYPE_POSITION.search(line[: struct.start(1)]):
            violations.add(path, lineno, "colour-literal",
                           "an Rgba/Hsla struct literal is a colour rise-theme should own")
        elif COLOUR_HEX.search(strip_comments(raw)):
            violations.add(path, lineno, "colour-literal",
                           "hex colours belong in tokens/palette.rs")

        match = LENGTH_CALL.search(line)
        if match and float(match.group(2).rstrip(".")) != 0.0:
            violations.add(path, lineno, "length-literal",
                           f"{match.group(1)}({match.group(2)}) hard-codes a length "
                           "that density cannot scale")

        match = BOX_STEP.search(line)
        if match and match.group(1) != "0":
            violations.add(path, lineno, "baked-scale",
                           f"the _{match.group(1)}() helper bakes tailwind's scale, not ours")
        match = CORNER_STEP.search(line) or TEXT_STEP.search(line)
        if match:
            violations.add(path, lineno, "baked-scale",
                           f"_{match.group(1)}() is a tailwind size step; ask the theme instead")
        match = BORDER_STEP.search(line)
        if match and match.group(1) not in ("0", "1"):
            violations.add(path, lineno, "baked-scale",
                           f"border width {match.group(1)} is a design decision; "
                           "only the 1px hairline is structural")


def check() -> int:
    violations = Violations()
    for path in sources():
        rel = path.relative_to(ROOT).as_posix()
        if is_exempt(rel):
            continue
        scan(path, violations)
    return violations.report()


if __name__ == "__main__":
    if not (ROOT / "crates").is_dir():
        print(f"no crates directory at {ROOT / 'crates'}", file=sys.stderr)
        raise SystemExit(2)
    raise SystemExit(check())
