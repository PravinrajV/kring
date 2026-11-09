// Deploys Kring to NEAR testnet under MASTER. Safe to re-run: every step checks what's already there.
//   node scripts/testnet/deploy.mjs
import { readFileSync, writeFileSync, mkdirSync, existsSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { sleep, MASTER, connectAll, viewAccount, accessKeys, view, call, near, transactions, utils, TGAS } from './lib.mjs';

const RES = new URL('../../res/', import.meta.url);
const OUT = new URL('../../deployments/testnet.json', import.meta.url);
const N = 20;
const TUSD = `tusd.${MASTER}`;
const CIRCLE = `circle.${MASTER}`;
const KEEPER = `keeper.${MASTER}`;
const members = Array.from({ length: N }, (_, i) => `m${String(i + 1).padStart(2, '0')}.${MASTER}`);
const level = (i) => (i % 4) + 1;
const UNIT = 250_000_000n;
const config = {
  period_secs: 180, waiting_periods: 1, probation_periods: 1, max_gift_periods: 24,
  vouch_window_secs: 1800, vouch_cooldown_periods: 12, proposal_secs: 180, gift_unit: UNIT.toString(),
};
const record = existsSync(OUT) ? JSON.parse(readFileSync(OUT)) : { network: 'testnet', txs: {} };
const save = () => { mkdirSync(new URL('.', OUT), { recursive: true }); writeFileSync(OUT, JSON.stringify(record, null, 2) + '\n'); };
const log = (s) => console.log(s);
const b58sha = (buf) => utils.serialize.base_encode(createHash('sha256').update(buf).digest());

const { account, publicKey } = await connectAll([TUSD, CIRCLE, KEEPER, ...members]);
const master = await account(MASTER);

async function ensureAccount(id, amount) {
  if (await viewAccount(id)) return;
  const out = await master.createAccount(id, publicKey, near(amount));
  record.txs[`create ${id.split('.')[0]}`] = out.transaction.hash;
  save();
  while (!(await viewAccount(id))) await sleep(500);
  log(`created ${id}`);
}

// 1. Test token
const tusdCode = readFileSync(new URL('kring_tusd.wasm', RES));
await ensureAccount(TUSD, '1.8');
if ((await viewAccount(TUSD)).code_hash !== b58sha(tusdCode)) {
  const t = await account(TUSD);
  const out = await t.signAndSendTransaction({ receiverId: TUSD, actions: [
    transactions.deployContract(tusdCode), transactions.functionCall('new', {}, 30n * TGAS, 0n) ] });
  record.txs['deploy+init tusd'] = out.transaction.hash;
  save();
  log(`deployed tUSD to ${TUSD}`);
}

// 2. Members and a keeper, each with a little NEAR for gas
// Prepaid gas is reserved at a pessimistic price, so the keeper (who settles) and m01 (the demo claimant) get more.
const topUp = async (id, min) => {
  const a = await viewAccount(id);
  if (BigInt(a.amount) < near(min)) {
    await master.sendMoney(id, near(min) - BigInt(a.amount));
    log(`topped up ${id} to ${min} NEAR`);
  }
};
for (const id of [KEEPER, ...members]) await ensureAccount(id, '0.1');
await topUp(KEEPER, '0.5');
await topUp(members[0], '0.3');

// 3. Circle: deploy, initialize and delete its only key in one atomic transaction
const circleCode = readFileSync(new URL('kring_circle.wasm', RES));
await ensureAccount(CIRCLE, '3.5');
const cAcc = await viewAccount(CIRCLE);
if (cAcc.code_hash !== b58sha(circleCode)) {
  if ((await accessKeys(CIRCLE)).length === 0) throw new Error(`${CIRCLE} is already keyless with different code; upgrades need a member vote`);
  const c = await account(CIRCLE);
  const out = await c.signAndSendTransaction({ receiverId: CIRCLE, actions: [
    transactions.deployContract(circleCode),
    transactions.functionCall('new', {
      token: TUSD,
      founders: members.map((id, i) => ({ account_id: id, level: level(i) })),
      config,
    }, 100n * TGAS, 0n),
    transactions.deleteKey(publicKey),
  ] });
  record.txs['deploy+init+delete key circle'] = out.transaction.hash;
  save();
  log(`deployed the circle to ${CIRCLE} and deleted its key in the same transaction`);
}

// 4. Token storage for the circle, then members register, mint and deposit
const bounds = await view(TUSD, 'storage_balance_bounds');
const reg = async (payer, who) => {
  if (await view(TUSD, 'storage_balance_of', { account_id: who })) return;
  await call(payer, TUSD, 'storage_deposit', { account_id: who }, { gas: 30n, deposit: BigInt(bounds.min) });
};
await reg(master, CIRCLE);
for (let i = 0; i < N; i++) {
  const m = await account(members[i]);
  await reg(m, members[i]);
  const info = await view(CIRCLE, 'get_member', { account_id: members[i] });
  if (BigInt(info.deposited) === 0n) {
    if (BigInt(await view(TUSD, 'ft_balance_of', { account_id: members[i] })) === 0n) await call(m, TUSD, 'mint', { amount: '10000000000' }, { gas: 30n });
    const amount = (BigInt(level(i)) * UNIT / 20n) * 12n; // a year of deposits up front, so the demo can run unattended
    const r = await call(m, TUSD, 'ft_transfer_call', { receiver_id: CIRCLE, amount: amount.toString(), msg: '' }, { gas: 50n, deposit: 1n });
    if (i === 0) record.txs['first deposit (m01)'] = r.hash;
  }
  process.stdout.write(`\rmembers ready: ${i + 1}/${N}`);
}
log('');

// 5. Proof
await sleep(3000);
const keys = await accessKeys(CIRCLE, 'final');
const circle = await view(CIRCLE, 'get_circle');
Object.assign(record, {
  master: MASTER, token: TUSD, circle: CIRCLE, keeper: KEEPER, members,
  circle_code_hash: (await viewAccount(CIRCLE)).code_hash, circle_wasm_sha256_b58: b58sha(circleCode),
  circle_access_keys: keys.length, config, held_tusd: Number(BigInt(await view(CIRCLE, 'get_held'))) / 1e6,
  genesis_ns: circle.genesis_ns, checked_at: new Date().toISOString(),
});
save();
log(`circle ${CIRCLE}: ${keys.length} access keys, code hash ${record.circle_code_hash}, holds ${record.held_tusd} tUSD`);
if (keys.length !== 0) throw new Error('The circle still has an access key');
