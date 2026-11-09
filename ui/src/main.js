// Kring one-screen UI. Every number comes from NEAR testnet over public RPC; nothing is mocked.
// Members sign their own transactions through NEAR Wallet Selector. The page never sees a key.
import './polyfill.js';
import './style.css';
import '@near-wallet-selector/modal-ui/styles.css';
import { setupWalletSelector } from '@near-wallet-selector/core';
import { setupModal } from '@near-wallet-selector/modal-ui';
import { setupMyNearWallet } from '@near-wallet-selector/my-near-wallet';
import { setupMeteorWallet } from '@near-wallet-selector/meteor-wallet';
import { setupHereWallet } from '@near-wallet-selector/here-wallet';
import { setupIntearWallet } from '@near-wallet-selector/intear-wallet';
import { actionCreators } from '@near-js/transactions';

const params = new URLSearchParams(location.search);
const CIRCLE = params.get('circle') || 'circle.kring-app.testnet';
const RPCS = params.get('rpc') ? [params.get('rpc')] : ['https://test.rpc.fastnear.com', 'https://rpc.testnet.near.org'];
const EXPLORER = 'https://testnet.nearblocks.io';
const TGAS = 1_000_000_000_000n;
const BOND = 10_000_000_000_000_000_000_000n; // 0.01 NEAR, kept by the circle to pay for a proposal's storage

const $ = (id) => document.getElementById(id);
const esc = (s) => String(s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);
const tusd = (u, dp = 2) => (Number(BigInt(u)) / 1e6).toLocaleString('en-US', { minimumFractionDigits: dp, maximumFractionDigits: dp });
const short = (id) => (id.endsWith('.' + CIRCLE.split('.').slice(1).join('.')) ? id.split('.')[0] : id);
const acctLink = (id) => `<a href="${EXPLORER}/address/${encodeURIComponent(id)}" target="_blank" rel="noopener">${esc(short(id))}</a>`;
const txLink = (h, label = h.slice(0, 8) + '…') => `<a href="${EXPLORER}/txns/${encodeURIComponent(h)}" target="_blank" rel="noopener">${esc(label)}</a>`;
const pct = (bps) => `${(bps / 100).toFixed(bps % 100 ? 2 : 0)}%`;

// ---------- RPC ----------
let rpcIdx = 0;
async function rpc(method, p) {
  let last;
  for (let i = 0; i < RPCS.length * 2; i++) {
    const url = RPCS[(rpcIdx + i) % RPCS.length];
    try {
      const res = await fetch(url, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ jsonrpc: '2.0', id: 'kring', method, params: p }) });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const j = await res.json();
      if (j.error) throw new Error(j.error.data || j.error.message || JSON.stringify(j.error));
      rpcIdx = (rpcIdx + i) % RPCS.length;
      return j.result;
    } catch (e) { last = e; }
  }
  throw last;
}
// Every read in one refresh is pinned to the same final block, so totals compared across calls
// (like the books check) can't straddle a transaction.
let atBlock = null;
const ref = () => (atBlock ? { block_id: atBlock } : { finality: 'final' });
const view = async (contract, method, args = {}) => {
  const r = await rpc('query', { request_type: 'call_function', ...ref(), account_id: contract, method_name: method, args_base64: btoa(JSON.stringify(args)) });
  return JSON.parse(new TextDecoder().decode(new Uint8Array(r.result)));
};
const query = (request_type, account_id) => rpc('query', { request_type, ...ref(), account_id });

// ---------- state ----------
let S = null;        // last chain snapshot
let selector, modal, me = null;
// ?as=<account> previews a member's view without a wallet. Acting still needs that member's wallet.
const PREVIEW = params.get('as');
let signedIn = null;
me = PREVIEW;

async function load() {
  atBlock = (await rpc('block', { finality: 'final' })).header.height;
  const [c, members, reports, keys, acct] = await Promise.all([
    view(CIRCLE, 'get_circle'), view(CIRCLE, 'get_members'), view(CIRCLE, 'get_reports'),
    query('view_access_key_list', CIRCLE), query('view_account', CIRCLE),
  ]);
  const settledCount = Number(c.next_to_settle);
  const from = Math.max(0, settledCount - 100);
  const [settlements, proposals, held, balance] = await Promise.all([
    view(CIRCLE, 'get_settlements', { from, limit: 100 }),
    view(CIRCLE, 'get_proposals', { from: Math.max(0, c.proposals - 20), limit: 20 }),
    view(CIRCLE, 'get_held'), view(c.token, 'ft_balance_of', { account_id: CIRCLE }),
  ]);
  let mine = null;
  if (me) {
    const [tok, reg, near] = await Promise.all([
      view(c.token, 'ft_balance_of', { account_id: me }),
      view(c.token, 'storage_balance_of', { account_id: me }),
      query('view_account', me).catch(() => null),
    ]);
    mine = { tok, reg, near: near?.amount ?? '0', member: members.find((m) => m.account_id === me) || null };
  }
  S = { c, members, reports, keys: keys.keys, codeHash: acct.code_hash, block: atBlock, settlements, proposals, held, balance, mine, at: Date.now() };
  render();
}

// ---------- render ----------
function periodText(c, leftMs) {
  const len = c.config.period_secs >= 120 ? `${Math.round(c.config.period_secs / 60)}-min` : `${c.config.period_secs}s`;
  const secs = Math.ceil(leftMs / 1000);
  return `${len} "month" · next ${secs >= 60 ? `${Math.floor(secs / 60)}m ${secs % 60}s` : `${secs}s`}`;
}
function statusOf(m) {
  const r = S.reports.find((x) => x.account_id === m.account_id);
  const cur = Number(S.c.current_period);
  if (m.status.state === 'left') return ['Left', 'muted'];
  if (m.expelled) return ['Expelled', 'bad'];
  if (m.status.state === 'exiting') return [`Leaving after period ${m.status.after_period}`, 'muted'];
  if (r && !r.closes_from) {
    if (!r.confirmed_period) return [`Reported sick · ${r.vouched.length}/2 vouched`, 'warn'];
    if (cur < Number(r.first_gift_period)) return ['Sick · waiting period', 'warn'];
    return [`Sick · ${r.gifts_paid} gift${r.gifts_paid === 1 ? '' : 's'} so far`, 'warn'];
  }
  if (r && r.closes_from) return ['Back at work', 'ok'];
  if (!m.good_standing) return ['Behind on deposits', 'bad'];
  return ['Healthy, giving', 'ok'];
}

function render() {
  const { c } = S;
  const cur = Number(c.current_period), next = Number(c.next_to_settle);
  const backlog = cur - next;
  const periodMs = c.config.period_secs * 1000;
  const sinceGenesis = Number(BigInt(c.now_ns) - BigInt(c.genesis_ns)) / 1e6;
  const left = Math.max(0, periodMs - (sinceGenesis % periodMs));
  const books = BigInt(S.held) === BigInt(S.balance);

  $('live').innerHTML = `<i class="dot ok"></i><span>Live · block ${S.block.toLocaleString('en-US')}</span>`;
  $('circle-id').innerHTML = `<a href="${EXPLORER}/address/${encodeURIComponent(CIRCLE)}" target="_blank" rel="noopener">${esc(CIRCLE)}</a>`;
  $('s-pots').textContent = `${tusd(c.total_pots, 0)} tUSD`;
  $('s-pots-sub').innerHTML = books
    ? `Books match the token balance ✓`
    : `<span class="bad">Books ${tusd(S.held)} ≠ balance ${tusd(S.balance)}</span>`;
  $('s-cov').textContent = c.last_coverage_bps == null ? '–' : pct(c.last_coverage_bps);
  $('s-cov-sub').textContent = c.last_coverage_bps == null ? 'nothing settled yet'
    : c.last_coverage_bps >= 10000 ? 'every gift paid in full' : 'gifts scaled down evenly';
  $('s-members').textContent = String(c.active_members);
  $('s-members-sub').textContent = `${c.open_reports} open report${c.open_reports === 1 ? '' : 's'}${c.exiting_members ? ` · ${c.exiting_members} leaving` : ''}`;
  $('s-period').textContent = `#${cur}`;
  $('s-period-sub').textContent = periodText(c, left);
  $('s-keys').textContent = S.keys.length === 0 ? 'Nobody' : `${S.keys.length} key${S.keys.length === 1 ? '' : 's'}!`;
  $('s-keys').className = S.keys.length === 0 ? 'ok' : 'bad';
  $('s-keys-sub').innerHTML = `${S.keys.length} access keys · code <code title="${esc(S.codeHash)}">${esc(S.codeHash.slice(0, 10))}…</code>`;

  const maxRatio = 1;
  $('members').innerHTML = S.members
    .slice()
    .sort((a, b) => a.account_id.localeCompare(b.account_id))
    .map((m) => {
      const [label, cls] = statusOf(m);
      const dep = BigInt(m.deposited), pot = BigInt(m.pot);
      const ratio = dep > 0n ? Math.min(maxRatio, Number(pot * 1000n / dep) / 1000) : 0;
      return `<tr class="${m.account_id === me ? 'you' : ''}">
        <td>${acctLink(m.account_id)}${m.account_id === me ? ' <span class="pill tiny">you</span>' : ''}</td>
        <td class="r">${m.level}</td>
        <td class="r">${tusd(m.gift_per_period, 0)}</td>
        <td class="r">${tusd(m.pot, 2)}</td>
        <td><div class="bar" title="${(ratio * 100).toFixed(0)}% of the ${tusd(dep)} tUSD they paid in is still in their own pot"><span style="width:${(ratio * 100).toFixed(1)}%"></span></div></td>
        <td><span class="st ${cls}">${esc(label)}</span></td></tr>`;
    }).join('');

  $('reports').innerHTML = S.reports.length === 0 ? `<p class="muted">Nobody is off sick right now.</p>` : S.reports.map((r) => {
    const v = r.vouchers.map((x) => `<span class="chip ${r.vouched.includes(x) ? 'ok' : ''}">${r.vouched.includes(x) ? '✓ ' : ''}${esc(short(x))}</span>`).join('');
    const state = r.closes_from ? `closing from period ${r.closes_from}` : r.confirmed_period ? `confirmed in period ${r.confirmed_period}, gifts from period ${r.first_gift_period}` : `waiting for vouches until ${new Date(Number(BigInt(r.vouch_deadline_ns) / 1000000n)).toLocaleTimeString()}`;
    return `<div class="row"><div><b>${acctLink(r.account_id)}</b> <span class="muted">reported in period ${r.opened_period}</span><div class="muted small">${esc(state)}</div></div><div class="chips">${v}</div></div>`;
  }).join('');

  const withGifts = S.settlements.filter((s) => s.gifts.length).slice(-6).reverse();
  $('settle-note').textContent = `${next} period${next === 1 ? '' : 's'} settled${backlog > 0 ? ` · ${backlog} waiting` : ''}`;
  $('settlements').innerHTML = (backlog > 0 ? `<div class="notice">${backlog} finished period${backlog === 1 ? ' is' : 's are'} waiting to be settled. Anyone can call <code>settle()</code>; it pays each period once, in order.${me ? ' <button class="btn small" data-act="settle">Settle next period</button>' : ''}</div>` : '')
    + (withGifts.length === 0 ? `<p class="muted">No gifts paid yet.</p>` : withGifts.map((s) => `
      <div class="row"><div><b>Period ${s.period}</b> <span class="muted">· ${s.donors} donors · settled by ${acctLink(s.settled_by)}</span>
        <div class="small">${s.gifts.map((g) => `${esc(short(g.account_id))} got <b>${tusd(g.paid)}</b>${g.paid !== g.due ? ` of ${tusd(g.due)}` : ''} tUSD`).join(', ')}</div></div>
        <div class="cov"><div class="bar ${s.coverage_bps < 10000 ? 'warn' : ''}"><span style="width:${Math.min(100, s.coverage_bps / 100)}%"></span></div><span class="small">${pct(s.coverage_bps)} covered</span></div></div>`).join(''));

  const voters = c.active_members;
  $('proposals').innerHTML = S.proposals.length === 0 ? `<p class="muted">No proposals yet.</p>` : S.proposals.slice().reverse().map((p) => {
    const what = p.kind.type === 'admit' ? `Admit ${esc(short(p.kind.account_id))} at level ${p.kind.level}`
      : p.kind.type === 'expel' ? `Expel ${esc(short(p.kind.account_id))}` : `Upgrade code to <code>${esc(p.kind.code_hash.slice(0, 12))}…</code>`;
    const need = p.kind.type === 'upgrade' ? 'two thirds' : 'a majority';
    return `<div class="row"><div><b>#${p.id} ${what}</b><div class="muted small">proposed by ${esc(short(p.proposer))} · ${p.approvals.length} of ${voters} approve · needs ${need}${p.vetoed_by ? ` · vetoed by ${esc(short(p.vetoed_by))}` : ''}</div></div><span class="st ${p.status === 'executed' ? 'ok' : p.status === 'vetoed' ? 'bad' : 'warn'}">${p.status}</span></div>`;
  }).join('');

  renderMe();
}

function renderMe() {
  const box = $('me-body');
  if (!me) {
    box.innerHTML = `<p class="muted">Connect a NEAR testnet wallet to see your pot and act. Reading the circle needs no wallet.</p>`;
    return;
  }
  const { mine, c } = S;
  const nearBal = (Number(BigInt(mine.near)) / 1e24).toFixed(3);
  const head = `<div class="who"><div><b>${acctLink(me)}</b>${!signedIn ? ' <span class="pill tiny">preview, read-only</span>' : ''}</div><span class="muted small">${nearBal} NEAR for gas · ${tusd(mine.tok)} tUSD in wallet</span></div>`;
  if (!mine.member) {
    box.innerHTML = head + `<p>You're not in this circle. Circles are invite-only: a member proposes you, the others vote, and any one of them can veto.</p>`
      + (!mine.reg ? `<button class="btn" data-act="register">Register for test tUSD</button>` : BigInt(mine.tok) === 0n ? `<button class="btn ghost" data-act="mint">Get 1,000 test tUSD</button>` : '');
    return;
  }
  const m = mine.member;
  const r = S.reports.find((x) => x.account_id === me);
  const drawnFor = S.reports.filter((x) => !x.closes_from && x.vouchers.includes(me) && !x.vouched.includes(me) && !x.confirmed_period);
  const behind = BigInt(m.required_deposit_to_date) - BigInt(m.deposited);
  const btns = [];
  if (m.status.state === 'active') {
    btns.push(`<button class="btn" data-act="deposit" data-amount="${behind > 0n ? behind : BigInt(m.deposit_per_period)}">Deposit ${tusd(behind > 0n ? behind : m.deposit_per_period)} tUSD</button>`);
    if (!r) btns.push(`<button class="btn warn" data-act="report_sick">I'm too sick to work</button>`);
    if (r && !r.closes_from) btns.push(`<button class="btn ghost" data-act="report_recovered">I'm back at work</button>`);
    for (const x of drawnFor) btns.push(`<button class="btn" data-act="vouch" data-claimant="${esc(x.account_id)}">Vouch for ${esc(short(x.account_id))}</button>`);
    btns.push(`<button class="btn ghost" data-act="request_exit">Leave the circle</button>`);
  }
  const exitFinal = m.status.state === 'exiting' && Number(c.next_to_settle) > Number(m.status.after_period);
  if (BigInt(m.gift_balance) > 0n || exitFinal) btns.push(`<button class="btn" data-act="withdraw">Withdraw ${tusd(BigInt(m.gift_balance) + (exitFinal ? BigInt(m.pot) : 0n))} tUSD</button>`);
  for (const p of S.proposals.filter((p) => p.status === 'open' && !p.approvals.includes(me))) {
    btns.push(`<button class="btn ghost" data-act="approve" data-id="${p.id}">Approve #${p.id}</button>`);
    if (p.kind.type === 'admit') btns.push(`<button class="btn ghost" data-act="veto" data-id="${p.id}">Veto #${p.id}</button>`);
  }
  box.innerHTML = head + `
    <div class="mine">
      <div><span class="k">Your pot</span><b>${tusd(m.pot)}</b></div>
      <div><span class="k">Gift if sick</span><b>${tusd(m.gift_per_period, 0)}</b><span class="small muted">per period</span></div>
      <div><span class="k">Deposit</span><b>${tusd(m.deposit_per_period)}</b><span class="small muted">per period</span></div>
      <div><span class="k">Gifts to withdraw</span><b>${tusd(m.gift_balance)}</b></div>
    </div>
    <p class="small ${m.good_standing ? 'muted' : 'bad'}">${m.good_standing ? 'Deposits up to date.' : `You're ${tusd(behind)} tUSD behind, so you can't report sick until you catch up.`}</p>
    <div class="actions">${btns.join('')}</div>`;
}

// ---------- actions ----------
function toast(html, cls = '') {
  const t = $('toast');
  t.className = `show ${cls}`;
  t.innerHTML = html;
  clearTimeout(toast.t);
  toast.t = setTimeout(() => (t.className = ''), 9000);
}
const fc = (method, args, tgas, deposit = 0n) => actionCreators.functionCall(method, args, BigInt(tgas) * TGAS, deposit);

async function act(el) {
  const a = el.dataset.act;
  const c = S.c;
  let receiverId = CIRCLE, actions;
  switch (a) {
    case 'register': receiverId = c.token; actions = [fc('storage_deposit', { account_id: me }, 30, 1_250_000_000_000_000_000_000n)]; break;
    case 'mint': receiverId = c.token; actions = [fc('mint', { amount: '1000000000' }, 30)]; break;
    case 'deposit': receiverId = c.token; actions = [fc('ft_transfer_call', { receiver_id: CIRCLE, amount: el.dataset.amount, msg: '' }, 60, 1n)]; break;
    case 'report_sick': actions = [fc('report_sick', {}, 50)]; break;
    case 'vouch': actions = [fc('vouch', { claimant: el.dataset.claimant }, 30)]; break;
    case 'report_recovered': actions = [fc('report_recovered', {}, 30)]; break;
    case 'request_exit':
      if (!confirm('Leave the circle? You still give for the current period, and your pot comes back once it has been settled.')) return;
      actions = [fc('request_exit', {}, 30)]; break;
    case 'withdraw': actions = [fc('withdraw', {}, 60)]; break;
    case 'settle': actions = [fc('settle', {}, 150)]; break;
    case 'approve': actions = [fc('approve', { id: Number(el.dataset.id) }, 30)]; break;
    case 'veto': actions = [fc('veto', { id: Number(el.dataset.id) }, 30)]; break;
    default: return;
  }
  if (!signedIn || signedIn !== me) {
    toast(signedIn ? `You're signed in as ${esc(signedIn)}, not ${esc(me)}.` : 'Connect your wallet to sign this.', 'bad');
    if (!signedIn) modal?.show();
    return;
  }
  el.disabled = true;
  try {
    const wallet = await selector.wallet();
    const out = await wallet.signAndSendTransaction({ receiverId, actions });
    const h = out?.transaction?.hash || out?.transaction_outcome?.id;
    const fail = out?.status?.Failure;
    if (fail) toast(`Transaction failed: ${esc(JSON.stringify(fail).slice(0, 200))}`, 'bad');
    else toast(h ? `Done. ${txLink(h, 'View transaction')}` : 'Sent.', 'ok');
  } catch (e) {
    toast(esc(e?.message || String(e)).slice(0, 300), 'bad');
  } finally {
    el.disabled = false;
    load().catch(showError);
  }
}
document.addEventListener('click', (e) => {
  const el = e.target.closest('[data-act]');
  if (el && S) act(el);
});

function showError(e) {
  $('live').innerHTML = `<i class="dot bad"></i><span>RPC unreachable, retrying</span>`;
  console.error(e);
}

// ---------- wallet ----------
async function initWallet() {
  selector = await setupWalletSelector({
    network: 'testnet',
    modules: [setupMyNearWallet(), setupMeteorWallet(), setupHereWallet(), setupIntearWallet()],
  });
  modal = setupModal(selector, { contractId: CIRCLE, description: 'Kring runs on NEAR testnet. Use a testnet account; nothing here has real value.' });
  const sync = (accounts) => {
    signedIn = accounts.find((x) => x.active)?.accountId || accounts[0]?.accountId || null;
    me = signedIn || PREVIEW;
    $('wallet').textContent = signedIn ? `Disconnect ${short(signedIn)}` : 'Connect wallet';
    if (S) load().catch(showError);
  };
  sync(selector.store.getState().accounts);
  selector.store.observable.subscribe((st) => sync(st.accounts));
  $('wallet').addEventListener('click', async () => {
    if (!signedIn) return modal.show();
    const w = await selector.wallet();
    await w.signOut();
  });
  // Browser wallets come back with ?transactionHashes=… after signing.
  const back = params.get('transactionHashes');
  if (back) {
    toast(`Done. ${txLink(back.split(',').pop(), 'View transaction')}`, 'ok');
    params.delete('transactionHashes');
    history.replaceState(null, '', location.pathname + (params.toString() ? `?${params}` : ''));
  }
  if (params.get('errorCode')) toast(`Wallet: ${esc(params.get('errorMessage') || params.get('errorCode'))}`, 'bad');
}

$('rpc-note').textContent = `Circle ${CIRCLE} via ${RPCS[0].replace('https://', '')}.`;
load().catch(showError);
setInterval(() => load().catch(showError), 8000);
setInterval(() => { if (S) { // tick the countdown between refreshes
  const c = S.c; const periodMs = c.config.period_secs * 1000;
  const since = Number(BigInt(c.now_ns) - BigInt(c.genesis_ns)) / 1e6 + (Date.now() - S.at);
  $('s-period-sub').textContent = periodText(c, Math.max(0, periodMs - (since % periodMs)));
} }, 1000);
initWallet().catch((e) => { console.error(e); $('wallet').textContent = 'Wallets unavailable'; $('wallet').disabled = true; });
