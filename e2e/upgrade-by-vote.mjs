// Rehearses a member-voted code upgrade on a local NEAR sandbox, the only way a Kring circle's
// code can change once its keys are gone. Needs no accounts or keys of yours.
//   node e2e/upgrade-by-vote.mjs [old.wasm] [new.wasm]     (both default to res/kring_circle.wasm)
import nw from 'near-workspaces';
import { createRequire } from 'node:module';
import { readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import assert from 'node:assert/strict';
createRequire(import.meta.url)('@near-js/utils').Logger.overrideLogger(undefined);
process.env.NEAR_WORKSPACES_NO_LOGS = '1';
const { Worker, parseNEAR } = nw;

const RES = new URL('../res/', import.meta.url);
const oldCode = readFileSync(process.argv[2] ?? new URL('kring_circle.wasm', RES));
const newCode = readFileSync(process.argv[3] ?? new URL('kring_circle.wasm', RES));
const TGAS = 1_000_000_000_000n;
const UNIT = 250_000_000n;
const N = 20;
const say = (s) => console.log(s);
const hex = (b) => createHash('sha256').update(b).digest('hex');

const worker = await Worker.init();
let failed = false;
try {
  const root = worker.rootAccount;
  const p = worker.provider;
  const sub = (name, n = '5') => root.createSubAccount(name, { initialBalance: parseNEAR(n) });
  const tusd = await sub('tusd', '10');
  await tusd.deploy(readFileSync(new URL('kring_tusd.wasm', RES)));
  await tusd.call(tusd, 'new', {});
  const members = [];
  for (let i = 1; i <= N; i++) {
    const a = await sub(`m${String(i).padStart(2, '0')}`);
    await a.call(tusd, 'storage_deposit', {}, { attachedDeposit: parseNEAR('0.01') });
    await a.call(tusd, 'mint', { amount: '10000000000' });
    members.push(a);
  }
  const circle = await sub('circle', '10');
  await circle.batch(circle)
    .deployContract(oldCode)
    .functionCall('new', {
      token: tusd.accountId,
      founders: members.map((m, i) => ({ account_id: m.accountId, level: (i % 4) + 1 })),
      config: { period_secs: 60, waiting_periods: 1, probation_periods: 1, max_gift_periods: 24,
        vouch_window_secs: 600, vouch_cooldown_periods: 12, proposal_secs: 60, gift_unit: UNIT.toString() },
    }, { gas: (100n * TGAS).toString() })
    .transact();
  await members[0].call(tusd, 'storage_deposit', { account_id: circle.accountId }, { attachedDeposit: parseNEAR('0.01') });
  const pk = (await circle.getKey()).getPublicKey().toString();
  await circle.batch(circle).deleteKey(pk).transact();
  const keys = async () => (await p.query({ request_type: 'view_access_key_list', finality: 'optimistic', account_id: circle.accountId })).keys.length;
  assert.equal(await keys(), 0);
  for (let i = 0; i < N; i++) {
    await members[i].call(tusd, 'ft_transfer_call', { receiver_id: circle.accountId, amount: (BigInt((i % 4) + 1) * UNIT / 20n * 6n).toString(), msg: '' }, { attachedDeposit: '1', gas: (100n * TGAS).toString() });
  }
  const before = { members: await circle.view('get_members'), held: await circle.view('get_held') };
  say(`Keyless circle running ${hex(oldCode).slice(0, 12)}…, holding ${Number(BigInt(before.held)) / 1e6} tUSD`);

  // propose() takes a 0.01 NEAR bond in newer builds; older builds ignore an attached deposit only if payable,
  // so try with the bond first and fall back to none for a v1 circle.
  const kind = { type: 'upgrade', code_hash: hex(newCode) };
  let id;
  try { id = await members[0].call(circle, 'propose', { kind }, { attachedDeposit: parseNEAR('0.01') }); }
  catch { id = await members[0].call(circle, 'propose', { kind }); }
  for (let i = 1; i < 13; i++) await members[i].call(circle, 'approve', { id });
  const args = Buffer.concat([Buffer.from(new BigUint64Array([BigInt(id)]).buffer), newCode]);
  const raw = (who) => who.callRaw(circle, 'execute_upgrade', args, { gas: (300n * TGAS).toString() });
  const r13 = await raw(members[0]);
  assert.ok(JSON.stringify(r13.result.status).includes('two thirds'), 'thirteen of twenty must not be enough');
  say('13 of 20 approvals: upgrade refused (needs two thirds)');
  const wrong = Buffer.concat([Buffer.from(new BigUint64Array([BigInt(id)]).buffer), Buffer.from('not the approved code')]);
  await members[13].call(circle, 'approve', { id });
  const rWrong = await members[0].callRaw(circle, 'execute_upgrade', wrong, { gas: (300n * TGAS).toString() });
  assert.ok(JSON.stringify(rWrong.result.status).includes("doesn't match"), 'different code must be refused');
  say('14 of 20 approvals, but different code: refused');
  const ok = await raw(members[0]);
  assert.ok(ok.succeeded, `upgrade failed: ${JSON.stringify(ok.result.status)}`);
  const acct = await p.query({ request_type: 'view_account', finality: 'optimistic', account_id: circle.accountId });
  const { utils } = createRequire(import.meta.url)('near-api-js');
  assert.equal(acct.code_hash, utils.serialize.base_encode(createHash('sha256').update(newCode).digest()));
  say(`14 of 20 approvals with the approved code: upgraded to ${hex(newCode).slice(0, 12)}…`);

  const after = { members: await circle.view('get_members'), held: await circle.view('get_held') };
  assert.deepEqual(after.members.map((m) => [m.account_id, m.pot]), before.members.map((m) => [m.account_id, m.pot]));
  assert.equal(after.held, before.held);
  assert.equal(BigInt(after.held), BigInt(await tusd.view('ft_balance_of', { account_id: circle.accountId })));
  assert.equal(await keys(), 0);
  const props = await circle.view('get_proposals', {});
  assert.equal(props[Number(id)].status, 'executed');
  say('Every pot and the token balance are unchanged, the proposal is executed, and the circle still has zero keys');
  say('\nUPGRADE REHEARSAL PASSED');
} catch (e) {
  failed = true;
  console.error('\nUPGRADE REHEARSAL FAILED:', e?.stack ?? e);
} finally {
  await worker.tearDown();
  process.exit(failed ? 1 : 0);
}
