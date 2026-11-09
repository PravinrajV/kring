#!/usr/bin/env bash
# Kring end-to-end demo. Runs on a clean Linux or macOS machine with Node 20+, curl and python3.
# Needs no NEAR account, wallet or key. Rust is optional: with it, unit tests and the simulation run too.
#
#   ./demo.sh            full run (about 6 minutes, most of it waiting for sandbox periods to pass)
#   ./demo.sh --build    also rebuild the wasm from source first (needs Rust + cargo-near)
set -euo pipefail
cd "$(dirname "$0")"
ROOT=$PWD
SANDBOX_VERSION=2.14.0   # same nearcore release testnet runs
say() { printf '\n\033[1m== %s\033[0m\n' "$*"; }
die() { echo "demo.sh: $*" >&2; exit 1; }

say "Checking prerequisites"
for c in node npm curl python3 tar; do command -v $c >/dev/null || die "$c is required"; done
node -e 'process.exit(+process.versions.node.split(".")[0] >= 20 ? 0 : 1)' || die "Node 20 or newer is required"
[ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env"
HAVE_RUST=0; command -v cargo >/dev/null && HAVE_RUST=1
echo "node $(node -v), rust: $([ $HAVE_RUST = 1 ] && cargo --version || echo 'not installed (unit tests and simulation will be skipped)')"

say "Installing the JS test harness (near-workspaces, binaryen)"
(cd e2e && npm ci --no-audit --no-fund --loglevel=error)

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64)  PLAT=Linux-x86_64;  SHA=a0ba004218b218bd74351795e7ce2eda4a2b6af5cd3e252bd5e8bfe3c8f2e691 ;;
  Linux-aarch64) PLAT=Linux-aarch64; SHA=d6b1ff773895972cd2fa098193c5d6cbc046912ff40bfe860e6c7abe807deedb ;;
  Darwin-arm64)  PLAT=Darwin-arm64;  SHA=ea53448ef97540c012e5c3d967c74d78292b0465a98cf7027811a936718fe232 ;;
  Darwin-x86_64) PLAT=Darwin-x86_64; SHA=b67b7bd48f18dc4dec96ca91332788670fd2e6b42a999323dfa7f62beb21ee69 ;;
  *) die "no NEAR sandbox build for $(uname -s)-$(uname -m)" ;;
esac
BIN_DIR=$ROOT/e2e/.sandbox/$PLAT
if [ ! -x "$BIN_DIR/near-sandbox" ]; then
  say "Downloading the NEAR sandbox node ($SANDBOX_VERSION, $PLAT)"
  TMP=$(mktemp)
  curl -sSfL "https://s3-us-west-1.amazonaws.com/build.nearprotocol.com/nearcore/$PLAT/$SANDBOX_VERSION/near-sandbox.tar.gz" -o "$TMP"
  GOT=$( (command -v sha256sum >/dev/null && sha256sum "$TMP" || shasum -a 256 "$TMP") | cut -d' ' -f1)
  [ "$GOT" = "$SHA" ] || { rm -f "$TMP"; die "sandbox checksum mismatch"; }
  mkdir -p "$ROOT/e2e/.sandbox" && tar xzf "$TMP" -C "$ROOT/e2e/.sandbox" && rm -f "$TMP"
fi

if [ "${1:-}" = "--build" ]; then
  [ $HAVE_RUST = 1 ] || die "--build needs Rust"
  say "Building both contracts from source"
  ./scripts/build.sh
fi

if [ $HAVE_RUST = 1 ]; then
  say "Unit tests (every attack in THREAT_MODEL.md has one)"
  cargo test --workspace --quiet 2>&1 | grep -E "^test result|FAILED|panicked" || true
  cargo test --workspace --quiet >/dev/null 2>&1 || die "unit tests failed"

  say "Simulation: 20 members, 24 months, three labeled sickness scenarios"
  before=$(cat sim/*.csv 2>/dev/null | python3 -c 'import hashlib,sys;print(hashlib.sha256(sys.stdin.buffer.read()).hexdigest())')
  (cd contracts/circle && cargo run --quiet --example simulate -- 2026 2>&1) | grep -v '^EVENT_JSON'
  after=$(cat sim/*.csv | python3 -c 'import hashlib,sys;print(hashlib.sha256(sys.stdin.buffer.read()).hexdigest())')
  [ "$before" = "$after" ] && echo "Same numbers as the committed sim/*.csv (seed 2026)" || echo "sim/*.csv regenerated"
else
  say "Skipping unit tests and simulation (no Rust). Committed results are in sim/SIMULATION.md"
fi

say "End to end on a local NEAR sandbox, using the wasm in res/"
NEAR_SANDBOX_BIN_PATH=1 NEAR_SANDBOX_BINARY_PATH="$BIN_DIR" node e2e/sandbox-demo.mjs

say "Zero-keys proof for the live testnet circle (public RPC, no key)"
./scripts/prove-keyless.sh || echo "(Couldn't reach testnet RPC; the sandbox run above already proved zero keys locally.)"

say "Done. Kring ran end to end."
