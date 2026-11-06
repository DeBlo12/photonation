#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 The Photonation contributors
#
# Checks a built disk image the way someone receiving it would use it: mounts it,
# copies the app out, inspects what ships, and runs the CLI and the editor.
#
# Usage: packaging/macos/check-installer.sh <image.dmg> [folder-of-raw-photos]
#   With a folder, the editor opens it and the CLI exports its photos.

set -euo pipefail

# How long the editor must keep running after it starts.
readonly EDITOR_START_SECONDS=10
# How long it gets to show a photo and save a screenshot of it.
readonly EDITOR_TIMEOUT_SECONDS=60

if [[ $# -lt 1 || $# -gt 2 ]]; then
    echo "usage: $0 <image.dmg> [folder-of-raw-photos]" >&2
    exit 2
fi
image="$1"
photos="${2:-}"
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
version="$(sed -n 's/^version = "\(.*\)"$/\1/p' "$root/Cargo.toml" | head -n 1)"

work="$(mktemp -d)"
mount_point="$work/mounted"
mkdir "$mount_point"
cleanup() {
    hdiutil detach -quiet "$mount_point" 2>/dev/null || true
    rm -rf "$work"
}
trap cleanup EXIT

passed() { echo "ok: $1"; }
failed() {
    echo "FAILED: $1" >&2
    exit 1
}

hdiutil verify -quiet "$image" || failed "the image's checksum"
passed "image checksum"
hdiutil attach -quiet -nobrowse -readonly -mountpoint "$mount_point" "$image"
[[ "$(readlink "$mount_point/Applications")" == "/Applications" ]] ||
    failed "the Applications link"
passed "Applications link"

# As if dragged out, then with the flag a download puts on it removed.
app="$work/Photonation.app"
ditto "$mount_point/Photonation.app" "$app"
xattr -dr com.apple.quarantine "$app" 2>/dev/null || true
contents="$app/Contents"
resources="$contents/Resources"

codesign --verify --deep --strict "$app" || failed "the signature"
passed "signature"

for file in LICENSE NOTICE THIRD-PARTY-LICENSES.txt photonation.icns \
    licenses/OFL-Inter.txt licenses/OFL-Sora.txt licenses/CC-BY-SA-3.0-Lensfun.txt; do
    [[ -s "$resources/$file" ]] || failed "$file is missing"
done
passed "licence texts and icon"

plist_value() { /usr/libexec/PlistBuddy -c "Print :$1" "$contents/Info.plist"; }
[[ "$(plist_value CFBundleShortVersionString)" == "$version" ]] ||
    failed "Info.plist says $(plist_value CFBundleShortVersionString), Cargo.toml $version"
passed "version $version"

minimum="$(plist_value LSMinimumSystemVersion)"
for binary in photonation-desktop photonation; do
    built_for="$(otool -l "$contents/MacOS/$binary" | awk '/minos/ {print $2; exit}')"
    [[ "$built_for" == "$minimum" ]] ||
        failed "$binary needs macOS $built_for, Info.plist says $minimum"
    if otool -L "$contents/MacOS/$binary" | tail -n +2 | grep -vE '^\s+(/System/|/usr/lib/)'; then
        failed "$binary links libraries that aren't on every Mac"
    fi
done
passed "built for macOS $minimum, system libraries only"

local_paths="$(strings -a "$contents/MacOS/photonation-desktop" "$contents/MacOS/photonation" |
    grep -c "/Users/$(id -un)" || true)"
[[ "$local_paths" == 0 ]] || failed "$local_paths strings name this Mac's home folder"
passed "no paths from this Mac"

cli="$contents/MacOS/photonation"
[[ "$("$cli" --version | head -n 1)" == "photonation $version" ]] || failed "photonation --version"
passed "CLI runs"
if [[ -n "$photos" ]]; then
    "$cli" export "$photos" -o "$work/exported" --long-edge 1024 >/dev/null
    [[ -n "$(ls -A "$work/exported")" ]] || failed "the CLI exported nothing"
    passed "CLI exports"
fi

# Through Launch Services, as Finder starts it, with the hardened runtime on and
# an empty home folder, as on a Mac that has never run it.
editor="$contents/MacOS/photonation-desktop"
first_run_home="$work/home"
output="$work/editor.log"
mkdir "$first_run_home"
open -n --env HOME="$first_run_home" --stdout "$output" --stderr "$output" "$app"
sleep "$EDITOR_START_SECONDS"
pgrep -f "$editor" >/dev/null || failed "the editor quit on a first run: $(cat "$output")"
grep -q "rendering on" "$output" || failed "the editor found no way to draw: $(cat "$output")"
pkill -f "$editor"
passed "editor starts and keeps running on a first run"

# A screenshot needs the photo to load and macOS to draw a window that opens
# unfocused, so it only runs when there is a photo to show.
if [[ -n "$photos" ]]; then
    capture="$work/editor.png"
    open -n --env HOME="$first_run_home" --env PHOTONATION_SCREENSHOT="$capture" "$app" --args "$photos"
    for ((second = 0; second < EDITOR_TIMEOUT_SECONDS; second++)); do
        [[ -s "$capture" ]] && break
        sleep 1
    done
    pkill -f "$editor" 2>/dev/null || true
    [[ -s "$capture" ]] || failed "the editor drew no photo in $EDITOR_TIMEOUT_SECONDS s"
    passed "editor opens a photo"
fi

echo "All checks passed for $(basename "$image")"
