// End-to-end Kring run on a local NEAR sandbox (a real nearcore node on your machine).
// Needs no accounts, keys or tokens of yours. Everything here is demo data.
import nw from 'near-workspaces';
import { createRequire } from 'node:module';
// Keep the output readable: no per-receipt logs from the client library.
createRequire(import.meta.url)('@near-js/utils').Logger.overrideLogger(undefined);
process.env.NEAR_WORKSPACES_NO_LOGS = '1';
const { Worker, parseNEAR } = nw;
import { readFileSync } from 'node:fs';
import assert from 'node:assert/strict';

const RES = new URL('../res/', import.meta.url);
const TGAS = 1_000_000_000_000n;
const UNIT = 250_000_000n; // 250 tUSD, the level-1 gift per period
const N = 20;
const gas = (t) => (BigInt(t) * TGAS).toString();
const fmt = (u) => (Number(BigInt(u)) / 1e6).toLocaleString('en-US', { minimumFractionDigits: 2, maximumFractionDigits: 2 });
const short = (id) => id.split('.')[0];
const say = (s) => console.log(s);
const step = (s) => console.log(`\n== ${s}`);

const worker = await Worker.init();
let failed = false;
try {
  const root = worker.rootAccount;
  const p = worker.provider;
  const sub = (name, n = '5') => root.createSubAccount(name, { initialBalance: parseNEAR(n) });

  step('Deploy the test token (tUSD, no value) and create 20 demo members');
  const tusd = await sub('tusd', '10');
  await tusd.deploy(readFileSync(new URL('kring_tusd.wasm', RES)));
  await tusd.call(tusd, 'new', {});
  const register = (a, who) => a.call(tusd, 'storage_deposit', who ? { account_id: who } : {}, { attachedDeposit: parseNEAR('0.01') });
  const balance = async (a) => BigInt(await tusd.view('ft_balance_of', { account_id: a.accountId ?? a }));
  const members = [];
  for (let i = 1; i <= N; i++) {
    const a = await sub(`m${String(i).padStart(2, '0')}`);
    await register(a);
    await a.call(tusd, 'mint', { amount: '10000000000' }); // 10,000 tUSD each
    members.push(a);
  }
  const level = (i) => (i % 4) + 1; // levels 1 to 4: gifts of 250 to 1,000 tUSD a period
  const keeper = await sub('keeper');
  say(`tUSD at ${tusd.accountId}; members ${members[0].accountId} .. ${members[N - 1].accountId}`);

  step('Deploy the circle, initialize it in the same transaction, then delete its only key');
  const circle = await sub('circle', '10');
  const config = {
    period_secs: 60, waiting_periods: 1, probation_periods: 2, max_gift_periods: 24,
    vouch_window_secs: 600, vouch_cooldown_periods: 12, proposal_secs: 60, gift_unit: UNIT.toString(),
  };
  await circle.batch(circle)
    .deployContract(readFileSync(new URL('kring_circle.wasm', RES)))
    .functionCall('new', {
      token: tusd.accountId,
      founders: members.map((m, i) => ({ account_id: m.accountId, level: level(i) })),
      config,
    }, { gas: gas(100) })
    .transact();
  await register(keeper, circle.accountId);
  const pk = (await circle.getKey()).getPublicKey().toString();
  await circle.batch(circle).deleteKey(pk).transact();
  const keyCount = async () => (await p.query({ request_type: 'view_access_key_list', finality: 'optimistic', account_id: circle.accountId })).keys.length;
  say(`Access keys on ${circle.accountId}: ${await keyCount()}`);
  assert.equal(await keyCount(), 0, 'circle must have zero access keys');

  const view = (method, args = {}) => circle.view(method, args);
  const nowPeriod = async () => Number((await view('get_circle')).current_period);
  const waitForPeriod = async (target) => {
    while ((await nowPeriod()) < target) {
      await p.fastForward(30);
      await new Promise((r) => setTimeout(r, 1500));
    }
  };
  const settleAll = async () => {
    while ((await view('get_circle')).settle_ready) await keeper.call(circle, 'settle', {}, { gas: gas(250) });
  };
  const deposit = (m, amount) =>
    m.call(tusd, 'ft_transfer_call', { receiver_id: circle.accountId, amount: amount.toString(), msg: '' },
      { attachedDeposit: '1', gas: gas(100) });
  const books = async () => {
    const held = BigInt(await view('get_held'));
    const bal = await balance(circle);
    assert.equal(held, bal, `books (${held}) must equal the circle's token balance (${bal})`);
    return bal;
  };

  step('Everyone deposits 6 periods of their level into their own pot');
  for (let i = 0; i < N; i++) await deposit(members[i], (BigInt(level(i)) * UNIT / 20n) * 6n);
  say(`Circle holds ${fmt(await books())} tUSD, split over 20 pots`);

  step('An outsider tries to deposit; the circle hands it back');
  const stranger = await sub('stranger');
  await register(stranger);
  await stranger.call(tusd, 'mint', { amount: '1000000000' });
  await deposit(stranger, 1000000000n);
  assert.equal(await balance(stranger), 1000000000n);
  say('The stranger still has all 1,000 tUSD');

  step('Probation: m01 can report sick only from period 2');
  const claimant = members[0];
  await assert.rejects(claimant.call(circle, 'report_sick', {}), /probation/i);
  say('A report in period 0 is refused');
  await waitForPeriod(2);
  await claimant.call(circle, 'report_sick', {});
  const report = await view('get_report', { account_id: claimant.accountId });
  say(`m01 reported sick in period ${report.opened_period}. Drawn vouchers: ${report.vouchers.map(short).join(', ')}`);
  assert.ok(!report.vouchers.includes(claimant.accountId));

  step('A member who was not drawn tries to vouch and is refused');
  const notDrawn = members.find((m) => m !== claimant && !report.vouchers.includes(m.accountId));
  await assert.rejects(notDrawn.call(circle, 'vouch', { claimant: claimant.accountId }), /weren't drawn/);
  say(`${short(notDrawn.accountId)} was refused`);

  step('Both drawn vouchers vouch');
  for (const v of report.vouchers) await members.find((m) => m.accountId === v).call(circle, 'vouch', { claimant: claimant.accountId });
  const confirmed = await view('get_report', { account_id: claimant.accountId });
  say(`Confirmed in period ${confirmed.confirmed_period}; the first gift period is ${confirmed.first_gift_period}`);

  step('A keeper who is not a member settles each period once it has finished');
  await waitForPeriod(Number(confirmed.first_gift_period) + 2);
  await settleAll();
  const settlements = await view('get_settlements', { from: 0, limit: 50 });
  for (const s of settlements) {
    const g = s.gifts.map((x) => `${short(x.account_id)} ${fmt(x.paid)}`).join(', ') || 'no gifts';
    say(`period ${s.period}: demand ${fmt(s.demand)}, collected ${fmt(s.collected)}, coverage ${(s.coverage_bps / 100).toFixed(2)}%, ${g}`);
  }
  const paid = settlements.flatMap((s) => s.gifts).reduce((a, x) => a + BigInt(x.paid), 0n);
  assert.equal(paid, 2n * UNIT, 'two gift periods of 250 tUSD each');
  assert.equal(settlements.find((s) => Number(s.period) === Number(report.opened_period)).gifts.length, 0, 'the waiting period pays nothing');
  await assert.rejects(keeper.call(circle, 'settle', {}, { gas: gas(250) }), /hasn't finished/);
  say('Settling the current period early is refused');
  await books();

  step('m01 withdraws the gifts to their own wallet, then reports recovery');
  const before = await balance(claimant);
  await claimant.call(circle, 'withdraw', {}, { gas: gas(100) });
  const got = (await balance(claimant)) - before;
  assert.equal(got, paid);
  say(`m01 received ${fmt(got)} tUSD`);
  await claimant.call(circle, 'report_recovered', {});
  say('m01 is back at work, so gifts stop');

  step('Admission: one veto stops a proposal; otherwise it needs a majority after the veto window');
  const kind = { type: 'admit', account_id: stranger.accountId, level: 1 };
  const bond = { attachedDeposit: parseNEAR('0.01') }; // pays for the proposal's storage
  await assert.rejects(members[1].call(circle, 'propose', { kind }), /Attach 0.01 NEAR/);
  say('A proposal without the 0.01 NEAR storage bond is refused');
  const vetoed = await members[1].call(circle, 'propose', { kind }, bond);
  await members[2].call(circle, 'veto', { id: vetoed });
  await assert.rejects(keeper.call(circle, 'execute', { id: vetoed }), /./);
  say(`Proposal ${vetoed} was vetoed by m03 and can't be executed`);
  const pid = await members[1].call(circle, 'propose', { kind }, bond);
  for (let i = 2; i <= 11; i++) await members[i].call(circle, 'approve', { id: pid });
  await assert.rejects(keeper.call(circle, 'execute', { id: pid }), /Veto window still open/);
  say('Executing inside the veto window is refused');
  await waitForPeriod((await nowPeriod()) + 1);
  await keeper.call(circle, 'execute', { id: pid });
  const newbie = await view('get_member', { account_id: stranger.accountId });
  say(`${short(stranger.accountId)} admitted at level ${newbie.level} in period ${newbie.joined_period} with 11 of 20 votes`);

  step('m20 leaves; their pot comes back once the current period is settled');
  const leaver = members[N - 1];
  await leaver.call(circle, 'request_exit', {});
  await assert.rejects(leaver.call(circle, 'withdraw', {}, { gas: gas(100) }), /Nothing to withdraw/);
  say('Withdrawing before the period is settled is refused');
  await waitForPeriod((await nowPeriod()) + 1);
  await settleAll();
  const pot = BigInt((await view('get_member', { account_id: leaver.accountId })).pot);
  const lb = await balance(leaver);
  await leaver.call(circle, 'withdraw', {}, { gas: gas(100) });
  assert.equal((await balance(leaver)) - lb, pot);
  say(`m20 got their ${fmt(pot)} tUSD pot back and has left`);

  step('Final checks');
  const c = await view('get_circle');
  say(`Active members ${c.active_members}, pots ${fmt(c.total_pots)} tUSD, unwithdrawn gifts ${fmt(c.total_gift_balances)} tUSD`);
  say(`The circle's token balance matches its books: ${fmt(await books())} tUSD`);
  assert.equal(await keyCount(), 0);
  say('The circle still has zero access keys');
  say('\nE2E PASSED');
} catch (e) {
  failed = true;
  console.error('\nE2E FAILED:', e?.stack ?? e);
} finally {
  await worker.tearDown();
  process.exit(failed ? 1 : 0);
}
