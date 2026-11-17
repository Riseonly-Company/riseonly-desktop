#!/usr/bin/env bash
set -euo pipefail

app=${1:?usage: macos-sign.sh <App.app> <out>}
out=${2:?usage: macos-sign.sh <App.app> <out>}
entitlements="app/resources/macos/Riseonly.entitlements"
name=$(basename "$app" .app)
work=$(mktemp -d)
cleanup() {
    if [ -n "${keychain:-}" ]; then
        security delete-keychain "$keychain" >/dev/null 2>&1 || true
    fi
    rm -rf "$work"
}
trap cleanup EXIT

signed=false
if [ -n "${APPLE_CERTIFICATE_P12_BASE64:-}" ]; then
    keychain="$work/signing.keychain-db"
    password=$(openssl rand -hex 16)
    printf '%s' "$APPLE_CERTIFICATE_P12_BASE64" | base64 --decode > "$work/cert.p12"
    security create-keychain -p "$password" "$keychain"
    security set-keychain-settings -lut 3600 "$keychain"
    security unlock-keychain -p "$password" "$keychain"
    security import "$work/cert.p12" -k "$keychain" -P "${APPLE_CERTIFICATE_PASSWORD:-}" -T /usr/bin/codesign
    security set-key-partition-list -S apple-tool:,apple: -s -k "$password" "$keychain" >/dev/null
    existing=()
    while IFS= read -r line; do existing+=("$(printf '%s' "$line" | tr -d '" ')"); done < <(security list-keychains -d user)
    security list-keychains -d user -s "$keychain" "${existing[@]}"
    identity=${APPLE_SIGNING_IDENTITY:-$(security find-identity -v -p codesigning "$keychain" | awk -F'"' 'NR == 1 { print $2 }')}
    [ -n "$identity" ] || { echo "no signing identity in the certificate" >&2; exit 1; }

    sign() { codesign --force --timestamp --options runtime --keychain "$keychain" --sign "$identity" "$@"; }
    if [ -d "$app/Contents/Frameworks" ]; then
        find "$app/Contents/Frameworks" -type f \( -name '*.dylib' -o -perm -u+x \) -print0 |
            while IFS= read -r -d '' file; do sign "$file"; done
    fi
    for helper in "$app"/Contents/MacOS/*; do
        [ "$(basename "$helper")" = riseonly ] || sign "$helper"
    done
    sign --entitlements "$entitlements" "$app"
    codesign --verify --deep --strict --verbose=2 "$app"
    signed=true
else
    echo "::warning::no Apple certificate, the bundle keeps its ad hoc signature and is not notarized"
fi

notarize() {
    [ "$signed" = true ] && [ -n "${APPLE_NOTARY_KEY_P8_BASE64:-}" ] || return 1
    printf '%s' "$APPLE_NOTARY_KEY_P8_BASE64" | base64 --decode > "$work/notary.p8"
    xcrun notarytool submit "$1" --key "$work/notary.p8" --key-id "${APPLE_NOTARY_KEY_ID:?}" \
        --issuer "${APPLE_NOTARY_ISSUER_ID:?}" --wait --timeout 45m
}

ditto -c -k --keepParent "$app" "$work/notarize.zip"
if notarize "$work/notarize.zip"; then
    xcrun stapler staple "$app"
    notarized=true
else
    notarized=false
    [ "$signed" = true ] && echo "::warning::no notary key, the bundle is signed but not notarized"
fi

ditto -c -k --keepParent "$app" "$out.zip"

stage="$work/dmg"
mkdir -p "$stage"
cp -R "$app" "$stage/"
ln -s /Applications "$stage/Applications"
hdiutil create -volname "$name" -srcfolder "$stage" -ov -format UDZO "$out.dmg" >/dev/null
if [ "$signed" = true ]; then
    codesign --force --timestamp --keychain "$keychain" --sign "$identity" "$out.dmg"
    if [ "$notarized" = true ] && notarize "$out.dmg"; then
        xcrun stapler staple "$out.dmg"
    fi
fi
echo "signed=$signed notarized=$notarized"
