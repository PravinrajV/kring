// Shared testnet helpers. The signing key comes from 1Password at run time and stays in memory.
import { createRequire } from 'node:module';
import { spawnSync } from 'node:child_process';
const require = createRequire(new URL('../../e2e/package.json', import.meta.url));
const nearApi = require('near-api-js');
export const { KeyPair, transactions, utils } = nearApi;
require('@near-js/utils').Logger.overrideLogger(undefined); // no per-receipt log spam

export const MASTER = process.env.KRING_MASTER || 'kring-app.testnet';
export const RPC = process.env.NEAR_RPC || 'https://test.rpc.fastnear.com';
export const TGAS = 1_000_000_000_000n;
export const near = (n) => BigInt(utils.format.parseNearAmount(String(n)));
export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function loadKey() {
  if (process.env.KRING_TESTNET_SECRET) return KeyPair.fromString(process.env.KRING_TESTNET_SECRET);
  const ref = process.env.KRING_OP_REF || `op://${process.env.OP_VAULT || 'grok'}/NEAR testnet ${MASTER}/credential`;
  const r = spawnSync('op', ['read', ref], { encoding: 'utf8' });
  if (r.status !== 0) throw new Error(`Couldn't read the deployer key from 1Password (${ref})`);
  return KeyPair.fromString(r.stdout.trim());
}

export async function connectAll(ids) {
  const kp = loadKey();
  const keyStore = new nearApi.keyStores.InMemoryKeyStore();
  for (const id of [MASTER, ...ids]) await keyStore.setKey('testnet', id, kp);
  const conn = await nearApi.connect({ networkId: 'testnet', nodeUrl: RPC, keyStore });
  return { conn, publicKey: kp.getPublicKey(), account: (id) => conn.account(id) };
}

export async function rpc(method, params) {
  for (let attempt = 0; ; attempt++) {
    const res = await fetch(RPC, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }) });
    if (res.status === 429 && attempt < 8) { await sleep(1500 * (attempt + 1)); continue; }
    const j = await res.json();
    if (j.error) throw new Error(JSON.stringify(j.error));
    return j.result;
  }
}
export const viewAccount = (id, finality = 'optimistic') => rpc('query', { request_type: 'view_account', finality, account_id: id }).catch(() => null);
export const accessKeys = async (id, finality = 'optimistic') => (await rpc('query', { request_type: 'view_access_key_list', finality, account_id: id })).keys;
export async function view(contract, method, args = {}) {
  const r = await rpc('query', { request_type: 'call_function', finality: 'optimistic', account_id: contract, method_name: method, args_base64: Buffer.from(JSON.stringify(args)).toString('base64') });
  return JSON.parse(Buffer.from(r.result).toString());
}

/// Calls a method and returns { value, hash }. Retries only on rate limits and timeouts.
export async function call(account, contractId, methodName, args = {}, { gas = 100n, deposit = 0n } = {}) {
  for (let attempt = 0; ; attempt++) {
    try {
      const out = await account.functionCall({ contractId, methodName, args, gas: gas * TGAS, attachedDeposit: deposit });
      const value = nearApi.providers.getTransactionLastResult(out);
      return { value, hash: out.transaction.hash };
    } catch (e) {
      const msg = String(e?.message ?? e);
      if (attempt < 5 && /429|Too Many|timeout|TimeoutError|ETIMEDOUT|fetch failed|InvalidNonce/i.test(msg)) { await sleep(2000 * (attempt + 1)); continue; }
      throw e;
    }
  }
}
