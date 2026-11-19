#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
SETUP_ID=1p0WMHzieeurYWioH8gYDeJ6Ntdd36d-4   # Drive file id of MixerAnalyzer-Setup.exe (same id when replaced as a new version)
PATH=/opt/homebrew/opt/llvm/bin:$PATH cargo xwin build --release --target x86_64-pc-windows-msvc
mkdir -p dist
cp target/x86_64-pc-windows-msvc/release/mixer-analyzer.exe dist/MixerAnalyzer.exe
rm -f dist/MixerAnalyzer-win.zip dist/MixerAnalyzer-Setup.exe
makensis -DVERSION="$VERSION" packaging/installer.nsi
rm -f dist/latest.txt
if [ -n "$SETUP_ID" ]; then
  printf '%s\nhttps://drive.usercontent.google.com/download?id=%s&export=download&confirm=t\n' "$VERSION" "$SETUP_ID" > dist/latest.txt
else
  echo "SETUP_ID is empty; not writing latest.txt" >&2
fi
