// Settles every finished period on the live testnet circle. settle() is permissionless, so the
// keeper is just a funded testnet account with no special rights.
// With --deposits, the demo members first pay what they owe plus `--ahead` periods (default 12),
// the way real members pay in every month.
//   node scripts/testnet/keeper.mjs [--max 50] [--deposits [--ahead 12]]
import { readFileSync, writeFileSync } from 'node:fs';
import { connectAll, view, call } from './lib.mjs';

const OUT = new URL('../../deployments/testnet.json', import.meta.url);
const d = JSON.parse(readFileSync(OUT));
const i = process.argv.indexOf('--max');
const max = i > 0 ? Number(process.argv[i + 1]) : 1000;
const fmt = (u) => (Number(BigInt(u)) / 1e6).toFixed(2);
const ai = process.argv.indexOf('--ahead');
const ahead = BigInt(ai > 0 ? Number(process.argv[ai + 1]) : 12);
const { account } = await connectAll([d.keeper, ...d.members]);
const keeper = await account(d.keeper);
if (process.argv.includes('--deposits')) {
  let paid = 0;
  for (const m of await view(d.circle, 'get_members')) {
    if (m.status.state !== 'active') continue;
    const owe = BigInt(m.required_deposit_to_date) - BigInt(m.deposited) + BigInt(m.deposit_per_period) * ahead;
    if (owe <= 0n) continue;
    const a = await account(m.account_id);
    if (BigInt(await view(d.token, 'ft_balance_of', { account_id: m.account_id })) < owe) await call(a, d.token, 'mint', { amount: '10000000000' }, { gas: 30n });
    await call(a, d.token, 'ft_transfer_call', { receiver_id: d.circle, amount: owe.toString(), msg: '' }, { gas: 50n, deposit: 1n });
    paid++;
  }
  console.log(`${paid} members paid their deposits (${ahead} periods ahead)`);
}
let n = 0;
while (n < max && (await view(d.circle, 'get_circle')).settle_ready) {
  const r = await call(keeper, d.circle, 'settle', {}, { gas: 150n });
  const s = r.value;
  n++;
  if (s.gifts.length) {
    (d.keeper_gift_settles ??= []).push({ period: s.period, coverage_bps: s.coverage_bps, gifts: s.gifts, tx: r.hash });
    writeFileSync(OUT, JSON.stringify(d, null, 2) + '\n');
  }
  console.log(`period ${s.period}: demand ${fmt(s.demand)}, paid ${fmt(s.collected)} tUSD, coverage ${(s.coverage_bps / 100).toFixed(2)}%  tx ${r.hash}`);
}
const c = await view(d.circle, 'get_circle');
console.log(`settled ${n}; next to settle ${c.next_to_settle}, current period ${c.current_period}`);
