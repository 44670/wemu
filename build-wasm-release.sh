#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")"

cargo build --release --target wasm32-unknown-unknown

src="target/wasm32-unknown-unknown/release/wemu.wasm"
dst="web/wemu.wasm"

if [[ ! -f "$src" ]]; then
  echo "missing wasm artifact: $src" >&2
  exit 1
fi

mkdir -p web
cp "$src" "$dst"
echo "copied $src -> $dst"
