#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
A=aarch64-apple-darwin; X=x86_64-apple-darwin
cargo build --release --target $A
cargo build --release --target $X
rm -rf dist/MixerAnalyzer.app dist/MixerAnalyzer-mac.zip
mkdir -p dist/MixerAnalyzer.app/Contents/MacOS
lipo -create target/$A/release/mixer-analyzer target/$X/release/mixer-analyzer \
  -output dist/MixerAnalyzer.app/Contents/MacOS/mixer-analyzer
cp packaging/Info.plist dist/MixerAnalyzer.app/Contents/Info.plist
codesign -s - -f --deep dist/MixerAnalyzer.app
ditto -c -k --keepParent dist/MixerAnalyzer.app dist/MixerAnalyzer-mac.zip
zip -j dist/MixerAnalyzer-mac.zip README.md assets/Pretendard-LICENSE.txt
