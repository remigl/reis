#!/usr/bin/env bash
# Cross-compile the Rust core for Android and drop it into the Flutter app.
# Needs: Android NDK, `cargo install cargo-ndk`, rustup targets:
#   rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
set -euo pipefail
cd "$(dirname "$0")/.."
OUT=app/flutter/android/app/src/main/jniLibs
cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 -o "$OUT" build --release -p reis-core
ls -la "$OUT"/*/libreis_core.so
