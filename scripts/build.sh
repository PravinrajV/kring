#!/usr/bin/env bash
# Build both contracts to res/. Rust >= 1.87 puts bulk-memory ops (memory.copy/fill) into wasm,
# and NEAR's VM still rejects them, so we lower them to plain loops with binaryen afterwards.
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$PWD
[ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env"
command -v cargo-near >/dev/null || { echo "cargo-near is missing: cargo install cargo-near --locked"; exit 1; }
[ -x e2e/node_modules/.bin/wasm-opt ] || (cd e2e && npm ci --no-audit --no-fund --loglevel=error)
WASM_OPT=$ROOT/e2e/node_modules/.bin/wasm-opt
mkdir -p res
for c in circle tusd; do
  (cd "contracts/$c" && cargo near build non-reproducible-wasm --locked --out-dir "$ROOT/res" >/dev/null 2>&1)
  "$WASM_OPT" "res/kring_$c.wasm" --enable-bulk-memory --enable-sign-ext --enable-mutable-globals \
    --llvm-memory-copy-fill-lowering -Oz --strip-debug -o "res/kring_$c.wasm.tmp"
  mv "res/kring_$c.wasm.tmp" "res/kring_$c.wasm"
  if "$WASM_OPT" "res/kring_$c.wasm" --mvp-features --enable-sign-ext --enable-mutable-globals -o /dev/null 2>/dev/null; then
    echo "built res/kring_$c.wasm ($(wc -c < "res/kring_$c.wasm") bytes, no bulk-memory ops)"
  else
    echo "res/kring_$c.wasm still has bulk-memory ops"; exit 1
  fi
done
sha256sum res/*.wasm
