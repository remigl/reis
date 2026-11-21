#!/usr/bin/env bash
# Generate the platform folders (not versioned) and add the microphone permission.
#   PLATFORMS=android scripts/setup_flutter.sh   (default: all)
set -euo pipefail
cd "$(dirname "$0")/../app/flutter"
PLATFORMS="${PLATFORMS:-android,ios,linux,windows,macos}"
flutter create --project-name reis --org org.reis --platforms="$PLATFORMS" .
rm -rf test   # generated widget test targets the template app

M=android/app/src/main/AndroidManifest.xml
if [ -f "$M" ]; then
  if ! grep -q RECORD_AUDIO "$M"; then
    sed -i 's#<application#<uses-permission android:name="android.permission.RECORD_AUDIO"/>\n    <uses-permission android:name="android.permission.INTERNET"/>\n    <uses-permission android:name="android.permission.READ_MEDIA_AUDIO"/>\n    <uses-permission android:name="android.permission.READ_EXTERNAL_STORAGE" android:maxSdkVersion="32"/>\n    <application#' "$M"
  fi
  # native audio helpers (MediaStore listing, MediaCodec decoding, artwork)
  KT=$(find android/app/src/main -name MainActivity.kt | head -1)
  cp android_src/MainActivity.kt "${KT:-android/app/src/main/kotlin/org/reis/reis/MainActivity.kt}"
  sed -i 's#android:label="reis"#android:label="Reis"#' "$M"
  # record_android needs API 24+
  for G in android/app/build.gradle.kts android/app/build.gradle; do
    if [ -f "$G" ]; then sed -i -E 's/minSdk( =|Version)? *flutter\.minSdkVersion/minSdk = 24/' "$G"; fi
  done
fi

P=ios/Runner/Info.plist
if [ -f "$P" ] && ! grep -q NSMicrophoneUsageDescription "$P"; then
  sed -i 's#<dict>#<dict>\n\t<key>NSMicrophoneUsageDescription</key>\n\t<string>Reis écoute la musique pour la reconnaître, entièrement sur l appareil.</string>#' "$P"
fi

# model for on-device "types de sons" libraries
if [ ! -f assets/models/mn10_as.rnm ]; then
  (gh release download models-v1 -R remigl/reis -p mn10_as.rnm -D assets/models 2>/dev/null \
    || curl -fsSL -o assets/models/mn10_as.rnm https://github.com/remigl/reis/releases/download/models-v1/mn10_as.rnm) \
    || echo "warning: mn10_as.rnm not downloaded: the app will only build music libraries"
fi
flutter pub get
echo "ok — now run ../../scripts/build_android.sh then: flutter run"
