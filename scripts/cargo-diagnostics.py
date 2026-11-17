import json
import sys


def main():
    show_warnings = "--warnings" in sys.argv[1:]
    label = next((a.split("=", 1)[1] for a in sys.argv[1:] if a.startswith("--label=")), "")
    errors = warnings = 0
    for line in sys.stdin:
        line = line.strip()
        if not line.startswith("{"):
            continue
        try:
            item = json.loads(line)
        except ValueError:
            continue
        if item.get("reason") != "compiler-message":
            continue
        message = item.get("message") or {}
        level = message.get("level", "")
        text = message.get("message", "")
        if level.startswith("error"):
            if text.startswith("aborting due to") or not message.get("rendered"):
                continue
            errors += 1
            sys.stdout.write(message["rendered"])
        elif level == "warning":
            if text.endswith("warnings emitted") or text.endswith("warning emitted"):
                continue
            warnings += 1
            if show_warnings:
                sys.stdout.write(message.get("rendered") or "")
        sys.stdout.flush()
    prefix = f"{label}: " if label else ""
    sys.stdout.write(f"{prefix}{errors} errors, {warnings} warnings\n")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
