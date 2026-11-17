#!/usr/bin/env bash
# Generate the platform folders (not versioned) and add the microphone permission.
set -euo pipefail
cd "$(dirname "$0")/../app/flutter"
flutter create --project-name reis --org org.reis --platforms=android,ios,linux,windows,macos .
M=android/app/src/main/AndroidManifest.xml
grep -q RECORD_AUDIO "$M" || sed -i 's#<application#<uses-permission android:name="android.permission.RECORD_AUDIO"/>\n    <application#' "$M"
P=ios/Runner/Info.plist
grep -q NSMicrophoneUsageDescription "$P" || sed -i 's#<dict>#<dict>\n\t<key>NSMicrophoneUsageDescription</key>\n\t<string>Reis écoute la musique pour la reconnaître, entièrement sur l appareil.</string>#' "$P"
flutter pub get
echo "ok — now run ../../scripts/build_android.sh then: flutter run"
