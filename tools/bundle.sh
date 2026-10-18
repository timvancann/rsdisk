#!/usr/bin/env bash
# Builds target/RSdisk.app: the release binary, Info.plist and the icon.
# The icon is generated from bundle/icon.svg (needs resvg and Apple's iconutil).
set -euo pipefail
cd "$(dirname "$0")/.."

cargo build --release -p rsdisk

app=target/RSdisk.app
iconset=target/RSdisk.iconset
rm -rf "$app" "$iconset"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources" "$iconset"

for size in 16 32 128 256 512; do
    resvg -w $size -h $size bundle/icon.svg "$iconset/icon_${size}x${size}.png"
    resvg -w $((size * 2)) -h $((size * 2)) bundle/icon.svg "$iconset/icon_${size}x${size}@2x.png"
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/RSdisk.icns"
rm -rf "$iconset"

cp target/release/rsdisk "$app/Contents/MacOS/rsdisk"
cp bundle/Info.plist "$app/Contents/Info.plist"
# Signed with a fixed local certificate, so the Full Disk Access and Finder automation grants
# survive rebuilds (they're tied to the signature, and an ad hoc one changes every build).
# RSDISK_SIGN picks the certificate; without it, fall back to ad hoc and say what that costs.
identity="${RSDISK_SIGN:-murmur dev}"
if security find-identity -v -p codesigning | grep -qF "\"$identity\""; then
    codesign --force --sign "$identity" "$app" >/dev/null
else
    echo "No \"$identity\" signing certificate; signing ad hoc. macOS will ask for Full Disk" \
        "Access and Finder automation again after every install." >&2
    codesign --force --sign - "$app" >/dev/null
fi
echo "$app"
