// Upgrades the live, keyless testnet circle to res/kring_circle.wasm by member vote:
// one member proposes the code's sha256, 13 more approve (14 of 20 is two thirds), then anyone
// sends the code to execute_upgrade. Uses the demo members' testnet keys from 1Password.
//   node scripts/testnet/upgrade-by-vote.mjs
import { readFileSync, writeFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { connectAll, view, viewAccount, accessKeys, call, near, sleep, utils, TGAS } from './lib.mjs';

const OUT = new URL('../../deployments/testnet.json', import.meta.url);
const d = JSON.parse(readFileSync(OUT));
const save = () => writeFileSync(OUT, JSON.stringify(d, null, 2) + '\n');
const code = readFileSync(new URL('../../res/kring_circle.wasm', import.meta.url));
const hex = createHash('sha256').update(code).digest('hex');
const b58 = utils.serialize.base_encode(createHash('sha256').update(code).digest());

const onChain = (await viewAccount(d.circle)).code_hash;
if (onChain === b58) { console.log(`${d.circle} already runs ${b58}`); process.exit(0); }
const { account } = await connectAll(d.members);
const m = await Promise.all(d.members.map((id) => account(id)));
// Sending ~300 KB of code with 300 TGas needs more NEAR than a demo member holds, and anyone may
// execute a passed proposal, so the deployer account sends it. It has no special power here.
const executor = await account(d.master);
const voters = (await view(d.circle, 'get_circle')).active_members;
const need = Math.ceil((voters * 2) / 3);
const expired = (p) => Date.now() * 1e6 > Number(p.created_ns) + 2 * d.config.proposal_secs * 1e9;
const props = await view(d.circle, 'get_proposals', { from: 0, limit: 100 });
const open = props.find((p) => p.status === 'open' && p.kind.type === 'upgrade' && p.kind.code_hash === hex && !expired(p));
const rec = open && d.upgrade?.proposal_id === open.id ? d.upgrade : { from_code_hash: onChain, to_code_hash: b58, sha256_hex: hex, approvals: [] };

// v1 circles take no deposit on propose; v2 and later need the 0.01 NEAR bond.
let r;
if (open) r = { value: open.id, hash: rec.propose_tx };
else try { r = await call(m[0], d.circle, 'propose', { kind: { type: 'upgrade', code_hash: hex } }, { gas: 30n, deposit: near('0.01') }); }
catch (e) {
  if (!/deposit|payable/i.test(String(e?.message ?? e))) throw e;
  r = await call(m[0], d.circle, 'propose', { kind: { type: 'upgrade', code_hash: hex } }, { gas: 30n });
}
const id = r.value;
Object.assign(rec, { proposal_id: id, propose_tx: r.hash });
d.upgrade = rec; save();
console.log(`${open ? 'resuming' : 'm01 proposed'} upgrade #${id} to sha256 ${hex}  tx ${r.hash}`);

const have = new Set((await view(d.circle, 'get_proposals', { from: id, limit: 1 }))[0].approvals);
for (let i = 1; i < need; i++) {
  if (have.has(d.members[i])) continue;
  const a = await call(m[i], d.circle, 'approve', { id }, { gas: 30n });
  rec.approvals.push(a.hash); save();
  process.stdout.write(`\rapprovals: ${i + 1}/${voters} (need ${need})`);
}
console.log('');

const args = Buffer.concat([Buffer.from(new BigUint64Array([BigInt(id)]).buffer), code]);
const out = await executor.functionCall({ contractId: d.circle, methodName: 'execute_upgrade', args, gas: 300n * TGAS, attachedDeposit: 0n });
const failure = out.status?.Failure ?? out.receipts_outcome.find((o) => o.outcome.status.Failure)?.outcome.status.Failure;
if (failure) throw new Error(`execute_upgrade failed: ${JSON.stringify(failure)}`);
rec.execute_tx = out.transaction.hash; save();
for (let i = 0; i < 20 && (await viewAccount(d.circle, 'final')).code_hash !== b58; i++) await sleep(1500);
const keys = await accessKeys(d.circle, 'final');
const now = (await viewAccount(d.circle, 'final')).code_hash;
Object.assign(d, { circle_code_hash: now, circle_wasm_sha256_b58: b58, circle_access_keys: keys.length });
rec.checked_at = new Date().toISOString(); save();
console.log(`executed  tx ${out.transaction.hash}\n${d.circle} now runs ${now} with ${keys.length} access keys`);
if (now !== b58 || keys.length !== 0) process.exit(1);
