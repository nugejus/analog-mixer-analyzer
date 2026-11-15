#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release --target x86_64-apple-darwin
cargo build --release --target aarch64-apple-darwin
rm -f dist/MixerAnalyzer-mac*.zip
# ponytail: one zip per chip; the universal zip is 13 MB, over the 10 MB browser upload limit. Go universal (lipo) if uploads stop being size-capped.
for pair in x86_64-apple-darwin:Intel aarch64-apple-darwin:AppleSilicon; do
  T=${pair%%:*}; N=${pair##*:}
  rm -rf dist/MixerAnalyzer.app
  mkdir -p dist/MixerAnalyzer.app/Contents/MacOS
  cp target/$T/release/mixer-analyzer dist/MixerAnalyzer.app/Contents/MacOS/mixer-analyzer
  cp packaging/Info.plist dist/MixerAnalyzer.app/Contents/Info.plist
  codesign -s - -f --deep dist/MixerAnalyzer.app
  ditto -c -k --keepParent dist/MixerAnalyzer.app dist/MixerAnalyzer-mac-$N.zip
  zip -j dist/MixerAnalyzer-mac-$N.zip README.md assets/Pretendard-LICENSE.txt
done
