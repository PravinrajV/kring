// Runs the story on the live testnet circle with real transactions:
//   1. the claimant tops up any deposits they're behind on, then reports sick
//   2. the two members the contract drew vouch (nobody else can)
//   3. the keeper settles each finished period; gifts start after the one-period wait
//   4. the claimant withdraws the gifts to their own wallet and reports recovered
// If the claimant already has a confirmed report, it picks up at step 3.
//   node scripts/testnet/live-scenario.mjs [m02] [--gifts 2]
// Each run is recorded in deployments/testnet.json under "runs".
import { readFileSync, writeFileSync } from 'node:fs';
import { connectAll, view, call, sleep } from './lib.mjs';

const OUT = new URL('../../deployments/testnet.json', import.meta.url);
const d = JSON.parse(readFileSync(OUT));
const save = () => writeFileSync(OUT, JSON.stringify(d, null, 2) + '\n');
const fmt = (u) => (Number(BigInt(u)) / 1e6).toFixed(2);
const argv = process.argv.slice(2);
const gi = argv.indexOf('--gifts');
const giftsWanted = gi >= 0 ? Number(argv[gi + 1]) : 2;
const short = argv.find((a, i) => !a.startsWith('--') && argv[i - 1] !== '--gifts') ?? 'm01';
const claimantId = short.includes('.') ? short : `${short}.${d.master}`;
if (!d.members.includes(claimantId)) throw new Error(`${claimantId} isn't one of the demo members`);

const { account } = await connectAll([d.keeper, ...d.members]);
const claimant = await account(claimantId);
const keeper = await account(d.keeper);
// Resume the claimant's unfinished run, if there is one, instead of starting a new record.
const prev = (d.runs ??= []).at(-1);
const run = prev && prev.claimant === claimantId && !prev.finished_at ? prev : { claimant: claimantId, started_at: new Date().toISOString(), txs: {} };
if (run !== prev) d.runs.push(run);
save();
// Load-balanced RPC nodes can lag a block behind the one that took the transaction.
const until = async (fn, ok) => { for (let i = 0; ; i++) { const v = await fn(); if (ok(v) || i >= 20) return v; await sleep(1000); } };
const getReport = () => view(d.circle, 'get_report', { account_id: claimantId });
const tx = (k, h) => { run.txs[k] = h; save(); };

const settleReady = async () => {
  while ((await view(d.circle, 'get_circle')).settle_ready) {
    const r = await call(keeper, d.circle, 'settle', {}, { gas: 150n });
    const s = r.value;
    if (s.gifts.length) (run.settles ??= []).push({ period: s.period, coverage_bps: s.coverage_bps, gifts: s.gifts, tx: r.hash });
    save();
    console.log(`keeper settled period ${s.period}: demand ${fmt(s.demand)}, paid ${fmt(s.collected)} tUSD, coverage ${(s.coverage_bps / 100).toFixed(2)}%  tx ${r.hash}`);
  }
};

await settleReady();
let report = await getReport();
if (report && report.closes_from) throw new Error(`${short}'s last report is still closing; run the keeper first`);
if (!report) {
  const m = await view(d.circle, 'get_member', { account_id: claimantId });
  const behind = BigInt(m.required_deposit_to_date) - BigInt(m.deposited);
  if (behind > 0n) {
    const top = behind + BigInt(m.deposit_per_period) * 2n; // a little ahead so the next period doesn't catch them out
    const r = await call(claimant, d.token, 'ft_transfer_call', { receiver_id: d.circle, amount: top.toString(), msg: '' }, { gas: 50n, deposit: 1n });
    tx('deposit', r.hash);
    console.log(`${short} deposited ${fmt(top)} tUSD to get up to date  tx ${r.hash}`);
  }
  const r = await call(claimant, d.circle, 'report_sick', {}, { gas: 50n });
  tx('report_sick', r.hash);
  report = await until(getReport, (x) => x);
  console.log(`${short} reported sick in period ${report.opened_period}; the contract drew ${report.vouchers.join(', ')}  tx ${r.hash}`);
}
for (const v of report.vouchers) {
  if (report.vouched.includes(v)) continue;
  const r = await call(await account(v), d.circle, 'vouch', { claimant: claimantId }, { gas: 30n });
  (run.txs.vouches ??= []).push(r.hash); save();
  console.log(`${v.split('.')[0]} vouched  tx ${r.hash}`);
}
report = await until(getReport, (x) => x?.confirmed_period);
if (!report?.confirmed_period) throw new Error('report not confirmed');
const lastGift = Number(report.first_gift_period) + Math.max(0, giftsWanted - report.gifts_paid - 1);
console.log(`confirmed in period ${report.confirmed_period}; gifts from period ${report.first_gift_period}; waiting for period ${lastGift} to finish`);
while (Number((await view(d.circle, 'get_circle')).current_period) <= lastGift) await sleep(10_000);
await settleReady();

const m = await view(d.circle, 'get_member', { account_id: claimantId });
if (BigInt(m.gift_balance) > 0n) {
  const before = BigInt(await view(d.token, 'ft_balance_of', { account_id: claimantId }));
  const r = await call(claimant, d.circle, 'withdraw', {}, { gas: 60n });
  let after = before;
  for (let i = 0; i < 10 && after === before; i++) { await sleep(1500); after = BigInt(await view(d.token, 'ft_balance_of', { account_id: claimantId })); }
  run.withdrawn_tusd = Number(after - before) / 1e6;
  tx('withdraw', r.hash);
  console.log(`${short} withdrew ${fmt(after - before)} tUSD to their own wallet  tx ${r.hash}`);
}
const rec = await call(claimant, d.circle, 'report_recovered', {}, { gas: 30n });
tx('report_recovered', rec.hash);
console.log(`${short} is back at work; gifts stop from this period  tx ${rec.hash}`);

const held = BigInt(await view(d.circle, 'get_held'));
const bal = BigInt(await view(d.token, 'ft_balance_of', { account_id: d.circle }));
run.books = { held_tusd: Number(held) / 1e6, token_balance_tusd: Number(bal) / 1e6, match: held === bal };
run.finished_at = new Date().toISOString(); save();
console.log(`books ${fmt(held)} tUSD, token balance ${fmt(bal)} tUSD: ${held === bal ? 'match' : 'MISMATCH'}`);
if (held !== bal) process.exit(1);
