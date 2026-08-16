#!/bin/sh
# SPDX-FileCopyrightText: 2026 Curtis Galloway
# SPDX-License-Identifier: Apache-2.0
#
# Submit one artifact to the Apple notary service, wait for the verdict,
# and staple the ticket on success.
#
#   macos/notarize.sh <artifact-to-submit> <path-to-staple>
#
# The two paths differ for an app bundle: the notary service only
# accepts .zip/.dmg/.pkg uploads, so an app goes up zipped but the
# ticket is stapled onto the bundle itself. For a DMG both are the
# same file.
#
# Credentials come from the environment (App Store Connect API key):
#   NOTARY_KEY_FILE   path to the AuthKey_<KEYID>.p8
#   NOTARY_KEY_ID     the key's ID
#   NOTARY_ISSUER_ID  the issuer UUID
#
# On rejection this dumps the notary log, which is the only place that
# names the offending executable (almost always one missing the
# hardened runtime or a secure timestamp).

set -e

SUBMIT="$1"
STAPLE="$2"

if [ -z "$SUBMIT" ] || [ -z "$STAPLE" ]; then
    echo "usage: macos/notarize.sh <artifact-to-submit> <path-to-staple>" >&2
    exit 1
fi

for var in NOTARY_KEY_FILE NOTARY_KEY_ID NOTARY_ISSUER_ID; do
    eval "value=\${$var:-}"
    if [ -z "$value" ]; then
        echo "notarize: $var is not set" >&2
        exit 1
    fi
done

RESULT="$(xcrun notarytool submit "$SUBMIT" \
    --key "$NOTARY_KEY_FILE" \
    --key-id "$NOTARY_KEY_ID" \
    --issuer "$NOTARY_ISSUER_ID" \
    --wait --timeout 30m --output-format json)"
echo "$RESULT"

# `notarytool submit --wait` has been known to exit 0 on a rejected
# submission, so the verdict is read out of the JSON rather than
# inferred from the exit code. Getting this wrong is how an unsigned
# build ships without anyone noticing.
STATUS="$(printf '%s' "$RESULT" | jq -r '.status')"
if [ "$STATUS" != "Accepted" ]; then
    echo "notarize: $SUBMIT was rejected (status: $STATUS)" >&2
    xcrun notarytool log "$(printf '%s' "$RESULT" | jq -r '.id')" \
        --key "$NOTARY_KEY_FILE" \
        --key-id "$NOTARY_KEY_ID" \
        --issuer "$NOTARY_ISSUER_ID" >&2 || true
    exit 1
fi

xcrun stapler staple "$STAPLE"
xcrun stapler validate "$STAPLE"
echo "notarized and stapled $STAPLE"
