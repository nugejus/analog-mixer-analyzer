#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
SETUP_ID=   # Drive file id of MixerAnalyzer-Setup.exe; fill after the first upload
PATH=/opt/homebrew/opt/llvm/bin:$PATH cargo xwin build --release --target x86_64-pc-windows-msvc
mkdir -p dist
cp target/x86_64-pc-windows-msvc/release/mixer-analyzer.exe dist/MixerAnalyzer.exe
rm -f dist/MixerAnalyzer-win.zip dist/MixerAnalyzer-Setup.exe
makensis -DVERSION="$VERSION" packaging/installer.nsi
rm -f dist/latest.txt
if [ -n "$SETUP_ID" ]; then
  printf '%s\nhttps://drive.google.com/uc?export=download&id=%s\n' "$VERSION" "$SETUP_ID" > dist/latest.txt
else
  echo "SETUP_ID가 비어 있어 latest.txt를 만들지 않습니다" >&2
fi
