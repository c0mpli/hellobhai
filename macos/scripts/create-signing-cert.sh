#!/bin/bash
# One-time setup: creates a self-signed "hellobhai Local Signing" code-signing
# certificate in your login keychain. build-and-run.sh signs every build with
# it, so macOS keeps mic / accessibility / screen recording permissions across
# rebuilds instead of asking again each time.
#
# The certificate is local only: not trusted system-wide, usable only by
# codesign, and removable any time from Keychain Access.
set -euo pipefail

IDENTITY_NAME="hellobhai Local Signing"

if security find-identity -p codesigning | grep -q "$IDENTITY_NAME"; then
  echo "\"$IDENTITY_NAME\" already exists — nothing to do."
  exit 0
fi

WORK_DIR=$(mktemp -d)
trap 'rm -rf "$WORK_DIR"' EXIT
P12_PASSWORD=$(uuidgen)

openssl req -x509 -newkey rsa:2048 -nodes -days 3650 \
  -keyout "$WORK_DIR/key.pem" -out "$WORK_DIR/cert.pem" \
  -subj "/CN=$IDENTITY_NAME" \
  -addext "extendedKeyUsage=critical,codeSigning" \
  -addext "keyUsage=critical,digitalSignature" \
  -addext "basicConstraints=critical,CA:false" 2>/dev/null

openssl pkcs12 -export -legacy -out "$WORK_DIR/identity.p12" \
  -inkey "$WORK_DIR/key.pem" -in "$WORK_DIR/cert.pem" -passout "pass:$P12_PASSWORD" 2>/dev/null \
  || openssl pkcs12 -export -out "$WORK_DIR/identity.p12" \
       -inkey "$WORK_DIR/key.pem" -in "$WORK_DIR/cert.pem" -passout "pass:$P12_PASSWORD"

security import "$WORK_DIR/identity.p12" -k ~/Library/Keychains/login.keychain-db \
  -P "$P12_PASSWORD" -T /usr/bin/codesign

echo "Created \"$IDENTITY_NAME\". Now run ./scripts/build-and-run.sh"
