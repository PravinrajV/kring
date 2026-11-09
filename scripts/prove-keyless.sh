#!/usr/bin/env bash
# Zero-keys proof, straight from public NEAR RPC. Needs curl and python3. No account, no key.
#   ./scripts/prove-keyless.sh [circle-account] [rpc-url]
set -euo pipefail
cd "$(dirname "$0")/.."
CIRCLE=${1:-circle.kring-app.testnet}
RPC=${2:-https://rpc.testnet.near.org}
q() { curl -sS --fail -X POST "$RPC" -H 'content-type: application/json' \
  -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"query\",\"params\":{\"request_type\":\"$1\",\"finality\":\"final\",\"account_id\":\"$CIRCLE\"}}"; }
KEYS=$(q view_access_key_list)
ACCT=$(q view_account)
python3 - "$CIRCLE" "$KEYS" "$ACCT" res/kring_circle.wasm <<'PY'
import hashlib, json, sys
circle, keys, acct, wasm = sys.argv[1], json.loads(sys.argv[2]), json.loads(sys.argv[3]), sys.argv[4]
if "error" in keys or "error" in acct:
    sys.exit(f"RPC error: {keys.get('error') or acct.get('error')}")
k = keys["result"]["keys"]
code_hash = acct["result"]["code_hash"]
A = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
n = int.from_bytes(hashlib.sha256(open(wasm, "rb").read()).digest(), "big")
local = ""
while n:
    n, r = divmod(n, 58)
    local = A[r] + local
print(f"account           {circle}")
print(f"block height      {keys['result']['block_height']} (final)")
print(f"access keys       {len(k)}")
print(f"on-chain code     {code_hash}")
print(f"res/ wasm sha256  {local}")
ok = len(k) == 0 and code_hash == local
print("RESULT            " + ("PASS: nobody holds a key to this account, and it runs the code in this repo" if ok else "FAIL"))
sys.exit(0 if ok else 1)
PY
