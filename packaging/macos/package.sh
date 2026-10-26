#!/bin/sh
set -eu
cd "$(dirname "$0")/../.."
cargo build --release
bundle=target/trigger.app/Contents
mkdir -p "$bundle/MacOS" "$bundle/Resources"
cp target/release/trigger "$bundle/MacOS/trigger"
cp packaging/macos/Info.plist "$bundle/Info.plist"
cp assets/trigger.icns "$bundle/Resources/trigger.icns"
