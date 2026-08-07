#!/usr/bin/env bash
set -euo pipefail

PROFILE="${1:-debug}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RES="$ROOT/app/resources/macos"
BIN="$ROOT/target/$PROFILE/riseonly"

# The environment the binary was COMPILED for, not the one the shell happens to
# have set. app_environment.rs reads option_env!("RISE_ENV") at build time and
# falls back to dev, so this has to fall back the same way — a bundle whose name
# says one environment and whose code talks to another is worse than either.
ENVIRONMENT="$(printf '%s' "${RISE_ENV:-dev}" | tr '[:upper:]' '[:lower:]')"
case "$ENVIRONMENT" in
    staging|stage)          ENVIRONMENT=staging ;;
    prod|production|release) ENVIRONMENT=prod ;;
    *)                       ENVIRONMENT=dev ;;
esac

case "$ENVIRONMENT" in
    dev)     APP_NAME="Riseonly Dev";     BUNDLE_ID="net.riseonly.desktop.dev";     SCHEME="riseonly-dev" ;;
    staging) APP_NAME="Riseonly Staging"; BUNDLE_ID="net.riseonly.desktop.staging"; SCHEME="riseonly-staging" ;;
    prod)    APP_NAME="Riseonly";         BUNDLE_ID="net.riseonly.desktop";         SCHEME="riseonly" ;;
esac

OUT="$ROOT/build/macos/$PROFILE/$APP_NAME.app"

[ -x "$BIN" ] || { echo "missing binary: $BIN — run cargo build -p riseonly first" >&2; exit 1; }

rm -rf "$OUT"
mkdir -p "$OUT/Contents/MacOS" "$OUT/Contents/Resources"

# The executable keeps ONE name across environments. It is what the crash
# reporter, `ps` and the single-instance lock see, and a space in it would have
# to be escaped by every one of them.
cp "$BIN" "$OUT/Contents/MacOS/riseonly"

sed -e "s|@APP_NAME@|$APP_NAME|g" \
    -e "s|@BUNDLE_ID@|$BUNDLE_ID|g" \
    -e "s|@URL_SCHEME@|$SCHEME|g" \
    "$RES/Info.plist.in" > "$OUT/Contents/Info.plist"
printf 'APPL????' > "$OUT/Contents/PkgInfo"

if [ -d "$ROOT/assets" ]; then
    cp -R "$ROOT/assets" "$OUT/Contents/Resources/assets"

    # Only the languages this build actually ships an interface for. The other
    # forty files stay in the repository — the catalogue needs all 41 because the
    # SERVER does — but bundling a locale the app cannot fully translate means
    # missing keys render as their own identifiers, which reads as a broken app.
    # The list is rise-i18n's SHIPPED, and its own test fails if they disagree.
    SHIPPED_LOCALES="$(sed -n 's/^pub const SHIPPED: &\[&str\] = &\[\(.*\)\];$/\1/p' \
        "$ROOT/crates/rise-i18n/src/shipped_languages.rs" | tr -d '" ' | tr ',' ' ')"
    if [ -n "$SHIPPED_LOCALES" ]; then
        for locale in "$OUT/Contents/Resources/assets/locales/"locale-*.json; do
            code="$(basename "$locale" .json)"; code="${code#locale-}"
            keep=no
            for shipped in $SHIPPED_LOCALES; do
                [ "$code" = "$shipped" ] && keep=yes
            done
            [ "$keep" = yes ] || rm -f "$locale"
        done
        echo "locales: $SHIPPED_LOCALES"
    else
        echo "warning: could not read SHIPPED from rise-i18n; bundling every locale" >&2
    fi
fi

# Built on demand from the 1024px master rather than committed as a binary, so
# there is exactly one source of truth for the mark.
ICON_MASTER="$RES/AppIcon.png"
ICON_OUT="$ROOT/build/macos/AppIcon.icns"
if [ -f "$ICON_MASTER" ]; then
    if [ ! -f "$ICON_OUT" ] || [ "$ICON_MASTER" -nt "$ICON_OUT" ]; then
        "$ROOT/scripts/make-icns.sh" "$ICON_MASTER" "$ICON_OUT"
    fi
    cp "$ICON_OUT" "$OUT/Contents/Resources/AppIcon.icns"
else
    echo "warning: no $ICON_MASTER; the bundle will show the generic app icon" >&2
fi

# Ad-hoc signature is enough for local runs and is what gives the bundle a
# stable Keychain ACL identity. Release signing is a separate, credentialed step.
codesign --force --sign - \
    --entitlements "$RES/Riseonly.entitlements" \
    --options runtime \
    --timestamp=none \
    "$OUT" 2>&1 | sed 's/^/codesign: /'

# Launch Services caches a bundle's name and icon by path. Without this a rename
# or a new icon shows up only after a logout, which reads as "the icon did not
# work" rather than "the cache is stale".
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister \
    -f "$OUT" >/dev/null 2>&1 || true
touch "$OUT"

echo "bundled: $OUT  ($ENVIRONMENT)"
