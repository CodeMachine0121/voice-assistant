#!/usr/bin/env bash
# Assembles target/<profile>/Voice Assistant.app by hand (no cargo-bundle needed).
# Usage: scripts/bundle.sh [debug|release]   (default: release)
set -euo pipefail

profile="${1:-release}"
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

if [[ "$(uname -s)" != "Darwin" || "$(uname -m)" != "arm64" ]]; then
  echo "Voice Assistant is built for macOS on Apple silicon (arm64)." >&2
  exit 1
fi
export MACOSX_DEPLOYMENT_TARGET=13.0

case "$profile" in
  release) cargo build --release -p voice-assistant ;;
  debug) cargo build -p voice-assistant ;;
  *) echo "profile must be debug or release" >&2; exit 2 ;;
esac

app="target/$profile/Voice Assistant.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "target/$profile/voice-assistant" "$app/Contents/MacOS/voice-assistant"
cp app/Info.plist "$app/Contents/Info.plist"

# Signing identity: set CODESIGN_IDENTITY to a certificate in your keychain (a
# self-signed "Code Signing" certificate is enough) so macOS keeps the microphone,
# accessibility and input-monitoring grants across rebuilds. Without one the app is
# signed ad hoc; that signature is tied to this exact binary, so every rebuild looks
# like a new app and the permissions must be granted again.
codesign --force --sign "${CODESIGN_IDENTITY:--}" "$app" >/dev/null
echo "Built $app"
