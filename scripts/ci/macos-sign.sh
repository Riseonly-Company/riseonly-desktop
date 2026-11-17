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

again() {
    local tries=$1 pause=$2 attempt=1
    shift 2
    until "$@"; do
        if [ "$attempt" -ge "$tries" ]; then
            echo "::error::$1 failed $tries times" >&2
            return 1
        fi
        echo "::warning::$1 failed, attempt $attempt of $tries, next in ${pause}s" >&2
        attempt=$((attempt + 1))
        sleep "$pause"
    done
}

signed=false
if [ -n "${APPLE_CERTIFICATE_P12_BASE64:-}" ]; then
    keychain="$work/signing.keychain-db"
    password=$(openssl rand -hex 16)
    printf '%s' "$APPLE_CERTIFICATE_P12_BASE64" | base64 --decode > "$work/cert.p12"
    security create-keychain -p "$password" "$keychain"
    security set-keychain-settings -lut 7200 "$keychain"
    security unlock-keychain -p "$password" "$keychain"
    security import "$work/cert.p12" -k "$keychain" -P "${APPLE_CERTIFICATE_PASSWORD:-}" -T /usr/bin/codesign
    security set-key-partition-list -S apple-tool:,apple: -s -k "$password" "$keychain" >/dev/null
    existing=()
    while IFS= read -r line; do existing+=("$(printf '%s' "$line" | tr -d '" ')"); done < <(security list-keychains -d user)
    security list-keychains -d user -s "$keychain" "${existing[@]}"
    identity=${APPLE_SIGNING_IDENTITY:-$(security find-identity -v -p codesigning "$keychain" | awk -F'"' 'NR == 1 { print $2 }')}
    [ -n "$identity" ] || { echo "no signing identity in the certificate" >&2; exit 1; }

    sign() {
        again 5 20 codesign --force --timestamp --options runtime --keychain "$keychain" --sign "$identity" "$@" </dev/null
    }
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

notary() {
    xcrun notarytool "$@" --key "$work/notary.p8" --key-id "${APPLE_NOTARY_KEY_ID:?}" \
        --issuer "${APPLE_NOTARY_ISSUER_ID:?}"
}

notarize() {
    local file=$1 attempt id
    for attempt in 1 2 3; do
        if notary submit "$file" --wait --timeout 20m 2>&1 | tee "$work/notary.log"; then
            grep -q 'status: Accepted' "$work/notary.log" && return 0
            id=$(sed -n 's/^ *id: //p' "$work/notary.log" | head -n 1)
            [ -z "$id" ] || notary log "$id" || true
            echo "::error::Apple did not accept $(basename "$file")" >&2
            return 1
        fi
        echo "::warning::the notary service failed for $(basename "$file"), attempt $attempt of 3" >&2
        sleep 30
    done
    echo "::error::$(basename "$file") is not notarized" >&2
    return 1
}

pack_dmg() {
    hdiutil create -volname "$name" -srcfolder "$stage" -ov -format UDZO "$out.dmg" >/dev/null
}

notarized=false
if [ "$signed" = true ] && [ -n "${APPLE_NOTARY_KEY_P8_BASE64:-}" ]; then
    printf '%s' "$APPLE_NOTARY_KEY_P8_BASE64" | base64 --decode > "$work/notary.p8"
    ditto -c -k --keepParent "$app" "$work/notarize.zip"
    notarize "$work/notarize.zip"
    again 5 20 xcrun stapler staple "$app"
    notarized=true
elif [ "$signed" = true ]; then
    echo "::warning::no notary key, the bundle is signed but not notarized"
fi

ditto -c -k --keepParent "$app" "$out.zip"

stage="$work/dmg"
mkdir -p "$stage"
cp -R "$app" "$stage/"
ln -s /Applications "$stage/Applications"
again 3 10 pack_dmg
if [ "$signed" = true ]; then
    again 5 20 codesign --force --timestamp --keychain "$keychain" --sign "$identity" "$out.dmg"
    if [ "$notarized" = true ]; then
        notarize "$out.dmg"
        again 5 20 xcrun stapler staple "$out.dmg"
    fi
fi
echo "signed=$signed notarized=$notarized"
