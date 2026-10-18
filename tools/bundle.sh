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
# Ad-hoc signature, so macOS treats it as one app (Full Disk Access, Gatekeeper on this Mac).
codesign --force --sign - "$app" >/dev/null
echo "$app"
