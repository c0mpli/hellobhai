#!/bin/bash
# Builds hellobhai, signs it with the local "hellobhai Local Signing"
# certificate, and relaunches it. Signing with the same certificate every
# time keeps macOS permissions (mic, accessibility, screen recording) granted
# across rebuilds — ad-hoc builds lose them on every build.
#
# Logs go to build/hellobhai.log (tail -f it to watch).
set -euo pipefail
cd "$(dirname "$0")/.."

APP=build/DerivedData/Build/Products/Debug/hellobhai.app
LOG=build/hellobhai.log

xcodebuild -project leanring-buddy.xcodeproj -scheme leanring-buddy -configuration Debug \
  -derivedDataPath build/DerivedData CODE_SIGNING_ALLOWED=NO \
  `# One binary instead of a launcher + hellobhai.debug.dylib: macOS refuses` \
  `# to load a dylib whose signature doesn't match the re-signed app.` \
  ENABLE_DEBUG_DYLIB=NO \
  build | grep -E "error:|BUILD (SUCCEEDED|FAILED)"

codesign --force --deep --options runtime \
  --entitlements leanring-buddy/leanring-buddy.entitlements \
  -s "hellobhai Local Signing" "$APP"

pkill -x hellobhai || true
while pgrep -x hellobhai >/dev/null; do sleep 0.3; done

# open (not running the binary directly) so macOS attributes permission
# prompts to hellobhai rather than to the terminal.
: > "$LOG"
open --env NSUnbufferedIO=YES --stdout "$LOG" --stderr "$LOG" "$APP"
echo "hellobhai launched — logs: $LOG"
