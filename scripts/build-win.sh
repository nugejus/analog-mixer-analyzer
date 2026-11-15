#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
PATH=/opt/homebrew/opt/llvm/bin:$PATH cargo xwin build --release --target x86_64-pc-windows-msvc
mkdir -p dist
cp target/x86_64-pc-windows-msvc/release/mixer-analyzer.exe dist/MixerAnalyzer.exe
rm -f dist/MixerAnalyzer-win.zip
zip -j dist/MixerAnalyzer-win.zip dist/MixerAnalyzer.exe README.md assets/Pretendard-LICENSE.txt
