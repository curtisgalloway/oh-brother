#!/bin/sh
# Copyright 2026 Curtis Galloway
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

# Build the "Oh Brother.app" shell around the Rust binaries. Usage:
#   macos/build.sh            build into macos/build/
#   macos/build.sh install    build and copy to /Applications
#   macos/build.sh dmg        build, then wrap the app in a
#                             drag-to-Applications disk image
#                             (build/OhBrother-<version>-macos-arm64.dmg)
#   macos/build.sh pack-dmg   wrap the ALREADY-BUILT build/Oh Brother.app
#                             in that same image without rebuilding it
#
# `pack-dmg` exists for the release workflow: the notarization ticket
# has to be stapled onto the app *before* the image is created, or the
# copy the user drags to /Applications has no ticket of its own and
# needs to reach Apple on first launch. So CI runs build.sh, notarizes
# and staples, then packs. Locally, `dmg` does both back to back.
#
# The app is self-contained: the release `label-web` and `label`
# binaries are bundled into Contents/Resources/bin, so the installed
# app no longer needs the repo. Build needs only cargo + Xcode CLT.

set -e
cd "$(dirname "$0")"
REPO="$(cd .. && pwd)"
APP="build/Oh Brother.app"

# The version single-source is the Rust workspace (same rule as
# windows/build.ps1). Both the bundle's Info.plist and the DMG filename
# are stamped from this one value so they cannot drift apart.
VERSION="$(sed -n 's/^version = "\(.*\)"$/\1/p' "$REPO/rust/Cargo.toml" | head -1)"
if [ -z "$VERSION" ]; then
    echo "build.sh: could not read workspace.package version from rust/Cargo.toml" >&2
    exit 1
fi

# Sign with a Developer ID when the keychain has one: macOS keys the
# Bluetooth permission to the app's code identity, and an ad-hoc
# signature changes every rebuild, re-prompting after each install.
# Fall back to ad-hoc so a clone still builds without an Apple account.
# Override with OH_BROTHER_SIGN_IDENTITY=- (or a specific identity).
IDENTITY="${OH_BROTHER_SIGN_IDENTITY:-}"
if [ -z "$IDENTITY" ]; then
    IDENTITY="$(security find-identity -v -p codesigning 2>/dev/null \
        | awk -F'"' '/Developer ID Application/ {print $2; exit}')"
fi
[ -n "$IDENTITY" ] || IDENTITY="-"

# The notary service rejects any executable that lacks the hardened
# runtime or a secure timestamp. Ad-hoc signatures support neither, so
# these only go on when there is a real identity to sign with. They are
# deliberately unquoted at the call sites — they must word-split into
# separate argv entries, and must vanish entirely when empty.
HARDENED=""
TIMESTAMP=""
if [ "$IDENTITY" != "-" ]; then
    HARDENED="--options runtime"
    TIMESTAMP="--timestamp"
fi

build_app() {
    cargo build --release --manifest-path "$REPO/rust/Cargo.toml"

    rm -rf build
    mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources/bin"

    cp Info.plist "$APP/Contents/Info.plist"
    # Info.plist ships a 0.0.0 placeholder; the shipped value is stamped
    # from the workspace here. Before this, the plist was hand-maintained
    # while the DMG filename came from Cargo.toml, so a release could
    # (and did) carry two different version numbers.
    /usr/libexec/PlistBuddy \
        -c "Set :CFBundleVersion $VERSION" \
        -c "Set :CFBundleShortVersionString $VERSION" \
        "$APP/Contents/Info.plist" > /dev/null
    cp "$REPO/rust/target/release/label" \
       "$REPO/rust/target/release/label-web" \
       "$APP/Contents/Resources/bin/"

    cp AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"

    swiftc -O -o "$APP/Contents/MacOS/Oh Brother" main.swift \
        -framework Cocoa -framework WebKit

    # The bundled binaries must be signed before the app seals over
    # them, or macOS kills them on launch.
    # shellcheck disable=SC2086
    codesign --force $HARDENED $TIMESTAMP -s "$IDENTITY" \
        "$APP/Contents/Resources/bin/label" \
        "$APP/Contents/Resources/bin/label-web"
    # shellcheck disable=SC2086
    codesign --force $HARDENED $TIMESTAMP -s "$IDENTITY" "$APP"
    echo "built $APP (signed: $IDENTITY)"
}

pack_dmg() {
    if [ ! -d "$APP" ]; then
        echo "pack-dmg: no $APP — run macos/build.sh first" >&2
        exit 1
    fi
    STAGE="build/dmg-stage"
    rm -rf "$STAGE"
    mkdir -p "$STAGE"
    # ditto rather than cp: it preserves the extended attributes a
    # stapled notarization ticket rides on.
    ditto "$APP" "$STAGE/Oh Brother.app"
    ln -s /Applications "$STAGE/Applications"
    DMG="build/OhBrother-$VERSION-macos-arm64.dmg"
    rm -f "$DMG"
    hdiutil create -volname "Oh Brother" -srcfolder "$STAGE" \
        -format UDZO -quiet "$DMG"
    # Gatekeeper assesses the image itself, not only the app inside it,
    # so the DMG gets its own signature. No hardened runtime here — that
    # option applies to executables.
    # shellcheck disable=SC2086
    codesign --force $TIMESTAMP -s "$IDENTITY" "$DMG"
    echo "built $DMG (signed: $IDENTITY)"
}

case "${1:-}" in
    "")
        build_app
        ;;
    install)
        build_app
        rm -rf "/Applications/Oh Brother.app"
        ditto "$APP" "/Applications/Oh Brother.app"
        echo "installed to /Applications/Oh Brother.app"
        ;;
    dmg)
        build_app
        pack_dmg
        ;;
    pack-dmg)
        pack_dmg
        ;;
    *)
        echo "usage: macos/build.sh [install|dmg|pack-dmg]" >&2
        exit 1
        ;;
esac
