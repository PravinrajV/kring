#!/usr/bin/env python3
"""Checks every testnet transaction linked in README.md against public archival RPC: who sent it,
to whom, which method, and that it succeeded. Needs python3 only. No account, no key."""
import json, re, sys, urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
RPC = sys.argv[1] if len(sys.argv) > 1 else "https://archival-rpc.testnet.near.org"
hashes = sorted(set(re.findall(r"txns/([1-9A-HJ-NP-Za-km-z]{43,44})", (ROOT / "README.md").read_text())))

def status(h):
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": "EXPERIMENTAL_tx_status",
                       "params": {"tx_hash": h, "sender_account_id": "kring-app.testnet", "wait_until": "NONE"}}).encode()
    r = json.load(urllib.request.urlopen(urllib.request.Request(RPC, body, {"content-type": "application/json"}), timeout=30))
    if "result" not in r:
        return None, False
    t, res = r["result"]["transaction"], r["result"]
    acts = [a if isinstance(a, str) else (a["FunctionCall"]["method_name"] if "FunctionCall" in a else next(iter(a))) for a in t["actions"]]
    ok = ("SuccessValue" in res["status"] or "SuccessReceiptId" in res["status"]) and not any(
        "Failure" in o["outcome"]["status"] for o in res["receipts_outcome"])
    return f'{t["signer_id"]} -> {t["receiver_id"]}: {", ".join(acts)}', ok

bad = 0
for h in hashes:
    what, ok = status(h)
    bad += not ok
    print(f'{"OK  " if ok else "FAIL"} {h[:10]}…  {what or "not found"}')
print(f"{len(hashes)} transactions, {bad} failed")
sys.exit(1 if bad else 0)
