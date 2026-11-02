#!/usr/bin/env bash
set -euo pipefail

export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH"

echo "1/3 Building p2d-wasm (wasm32-unknown-unknown, release-wasm)..."
cargo build --profile release-wasm --target wasm32-unknown-unknown -p p2d-wasm

echo "2/3 Generating JS bindings via wasm-bindgen..."
mkdir -p extension/pkg
wasm-bindgen --target web --out-dir extension/pkg target/wasm32-unknown-unknown/release-wasm/p2d_wasm.wasm

echo "3/3 Optimizing WASM binary with wasm-opt..."
wasm-opt -O3 --enable-simd --enable-bulk-memory --enable-nontrapping-float-to-int \
  extension/pkg/p2d_wasm_bg.wasm -o extension/pkg/p2d_wasm_bg.wasm

ls -lh extension/pkg/p2d_wasm_bg.wasm
echo "Done! Load 'extension/' as an unpacked extension in chrome://extensions"
