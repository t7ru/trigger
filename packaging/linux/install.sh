#!/bin/sh
set -eu
cd "$(dirname "$0")/../.."
cargo build --release
data_dir="${XDG_DATA_HOME:-$HOME/.local/share}"
install -Dm755 target/release/trigger "$HOME/.local/bin/trigger"
install -Dm644 assets/icon.png "$data_dir/icons/hicolor/1024x1024/apps/trigger.png"
install -Dm644 packaging/linux/trigger.desktop "$data_dir/applications/trigger.desktop"
