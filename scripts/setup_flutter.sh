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
    sed -i 's#<application#<uses-permission android:name="android.permission.RECORD_AUDIO"/>\n    <application#' "$M"
  fi
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

flutter pub get
echo "ok — now run ../../scripts/build_android.sh then: flutter run"
