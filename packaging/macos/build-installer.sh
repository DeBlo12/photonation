#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 The Photonation contributors
#
# Builds the release binaries, Photonation.app and the macOS disk image
# (target/installer/photonation-<version>-macos-<architecture>.dmg) for this
# Mac's architecture. Needs Rust (rustup), the Xcode command line tools and
# cargo-about:
#   cargo install cargo-about --locked --features cli
#
# Usage: packaging/macos/build-installer.sh [--skip-build]
#   --skip-build  Package the binaries already in target/release.
#
# Without a signing identity the app is signed ad hoc, and macOS makes people
# who download it approve it by hand (docs/DEVELOPMENT.md). For a release set:
#   PHOTONATION_SIGN_IDENTITY   "Developer ID Application: <name> (<team>)"
#   PHOTONATION_NOTARY_PROFILE  a `xcrun notarytool store-credentials` profile,
#                               to notarize the disk image and staple the ticket

set -euo pipefail

# The oldest macOS the build targets; it also goes in the app's Info.plist.
readonly MINIMUM_MACOS=11.0

skip_build=false
case "${1:-}" in
    "") ;;
    --skip-build) skip_build=true ;;
    *)
        echo "usage: $0 [--skip-build]" >&2
        exit 2
        ;;
esac

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

# The workspace version, from [workspace.package] in Cargo.toml.
version="$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)"
architecture="$(uname -m)"
sign_identity="${PHOTONATION_SIGN_IDENTITY:--}"
notary_profile="${PHOTONATION_NOTARY_PROFILE:-}"

if [[ -n "$notary_profile" && "$sign_identity" == "-" ]]; then
    echo "PHOTONATION_NOTARY_PROFILE needs PHOTONATION_SIGN_IDENTITY: Apple only notarizes signed apps." >&2
    exit 1
fi
if ! command -v cargo-about >/dev/null; then
    echo "cargo-about not found. Install it with: cargo install cargo-about --locked --features cli" >&2
    exit 1
fi

installer_directory="target/installer"
stage="$installer_directory/stage"
app="$stage/Photonation.app"
volume_name="Photonation $version"
writable_image="$installer_directory/photonation-writable.dmg"
final_image="$installer_directory/photonation-$version-macos-$architecture.dmg"
volume="/Volumes/$volume_name"

if [[ -e "$volume" ]]; then
    echo "$volume is already mounted. Eject it and run this again." >&2
    exit 1
fi

if [[ "$skip_build" == false ]]; then
    MACOSX_DEPLOYMENT_TARGET="$MINIMUM_MACOS" \
        cargo build --release --locked -p photonation-desktop -p photonation-cli
fi

# The licence texts of every compiled-in crate (NOTICE, "Obligations when
# distributing binaries").
cargo about generate --locked --workspace --fail \
    -o target/THIRD-PARTY-LICENSES.txt packaging/third-party-licenses.hbs

# The bundle. The CLI sits beside the editor so one signature covers both.
rm -rf "$stage" "$writable_image" "$final_image"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources/licenses"
cp target/release/photonation-desktop target/release/photonation "$app/Contents/MacOS/"
sed -e "s/@VERSION@/$version/g" -e "s/@MINIMUM_MACOS@/$MINIMUM_MACOS/g" \
    packaging/macos/Info.plist >"$app/Contents/Info.plist"
plutil -lint -s "$app/Contents/Info.plist"
cp assets/icon/photonation.icns "$app/Contents/Resources/"
cp LICENSE NOTICE README.md target/THIRD-PARTY-LICENSES.txt "$app/Contents/Resources/"
cp assets/fonts/OFL-Inter.txt assets/fonts/OFL-Sora.txt "$app/Contents/Resources/licenses/"
cp crates/lens/lensfun/COPYING.CC_BY-SA_3.0 "$app/Contents/Resources/licenses/CC-BY-SA-3.0-Lensfun.txt"

# Inside out: the CLI, then the bundle around it. The hardened runtime is what
# notarization asks for; a secure timestamp needs a real identity.
sign() {
    local timestamp=--timestamp
    if [[ "$sign_identity" == "-" ]]; then
        timestamp=--timestamp=none
    fi
    codesign --force --options runtime "$timestamp" --sign "$sign_identity" "$@"
}
sign --identifier io.github.deblo12.photonation.cli "$app/Contents/MacOS/photonation"
sign "$app"
codesign --verify --deep --strict "$app"

ln -s /Applications "$stage/Applications"

# A writable image first, so Finder can save the window layout into it.
hdiutil create -quiet -fs HFS+ -volname "$volume_name" -srcfolder "$stage" \
    -format UDRW "$writable_image"
hdiutil attach -quiet -noverify -noautoopen "$writable_image"
trap 'hdiutil detach -quiet "$volume" 2>/dev/null || true' EXIT

# Lays out the window people see when they open the image: the app on the left,
# Applications on the right. Finder may need permission to be automated, and
# there may be no Finder at all on a build machine; the image works either way.
arrange_window() {
    osascript - "$volume_name" <<'APPLESCRIPT'
on run {volumeName}
    with timeout of 30 seconds
        tell application "Finder"
            tell disk volumeName
                open
                set current view of container window to icon view
                set toolbar visible of container window to false
                set statusbar visible of container window to false
                set the bounds of container window to {200, 120, 860, 520}
                set viewOptions to the icon view options of container window
                set arrangement of viewOptions to not arranged
                set icon size of viewOptions to 128
                set text size of viewOptions to 13
                set position of item "Photonation.app" of container window to {170, 170}
                set position of item "Applications" of container window to {490, 170}
                update without registering applications
                close
            end tell
        end tell
    end timeout
end run
APPLESCRIPT
}
if arrange_window; then
    # Finder writes the layout a moment after the window closes.
    for _ in {1..20}; do
        [[ -f "$volume/.DS_Store" ]] && break
        sleep 0.5
    done
    [[ -f "$volume/.DS_Store" ]] || echo "warning: Finder saved no window layout" >&2
else
    echo "warning: could not lay out the disk image window; it uses Finder's defaults" >&2
fi

rm -rf "$volume/.fseventsd"
sync
hdiutil detach -quiet "$volume"
trap - EXIT

hdiutil convert -quiet "$writable_image" -format ULMO -o "$final_image"
rm -f "$writable_image"

if [[ "$sign_identity" != "-" ]]; then
    codesign --force --timestamp --sign "$sign_identity" "$final_image"
fi
if [[ -n "$notary_profile" ]]; then
    xcrun notarytool submit "$final_image" --keychain-profile "$notary_profile" --wait
    xcrun stapler staple "$final_image"
fi

echo "$final_image"
