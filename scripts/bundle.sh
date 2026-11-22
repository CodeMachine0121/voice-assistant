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

# An ad-hoc signature gives macOS a stable identity to attach permissions to.
codesign --force --sign - "$app" >/dev/null
echo "Built $app"
