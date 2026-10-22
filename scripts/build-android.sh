#!/usr/bin/env bash
# Build a signed release APK of gauntlet-rs.
#   scripts/build-android.sh [aarch64-linux-android|x86_64-linux-android]
# Tablets/phones are aarch64; the emulator is x86_64.
# After installing, copy your disc image to Android/data/com.gauntletrs.game/files/
# (e.g. `adb push game.iso /sdcard/Android/data/com.gauntletrs.game/files/`).
set -euo pipefail
cd "$(dirname "$0")/../crates/gdl-android"
TARGET="${1:-aarch64-linux-android}"
export ANDROID_HOME="${ANDROID_HOME:-/opt/android-sdk}"
export ANDROID_NDK_ROOT="${ANDROID_NDK_ROOT:-$(ls -d "$ANDROID_HOME"/ndk/* | sort -V | tail -1)}"
KS="${GAUNTLET_KEYSTORE:-$HOME/.android/gauntlet-rs.keystore}"
if [ ! -f "$KS" ]; then
  keytool -genkeypair -keystore "$KS" -alias gauntlet -keyalg RSA -keysize 2048 -validity 36500 \
    -storepass gauntletrs -keypass gauntletrs -dname "CN=gauntlet-rs personal"
fi
export CARGO_APK_RELEASE_KEYSTORE="$KS"
export CARGO_APK_RELEASE_KEYSTORE_PASSWORD="${GAUNTLET_KEYSTORE_PASSWORD:-gauntletrs}"
cargo apk build --release --target "$TARGET"
ls -la ../../target/release/apk/*.apk
