// Creates a fresh testnet account through NEAR's free faucet helper.
// The private key goes straight into 1Password (vault "grok" by default) and is never printed.
//   node scripts/testnet/new-account.mjs kring-demo.testnet
import { createRequire } from 'node:module';
import { spawnSync } from 'node:child_process';
import { writeFileSync, rmSync, mkdtempSync } from 'node:fs';
import { join } from 'node:path';
const require = createRequire(new URL('../../e2e/package.json', import.meta.url));
const { KeyPair } = require('near-api-js');

const accountId = process.argv[2];
const vault = process.env.OP_VAULT || 'grok';
if (!accountId || !accountId.endsWith('.testnet')) throw new Error('usage: new-account.mjs <name>.testnet');

const kp = KeyPair.fromRandom('ed25519');
const publicKey = kp.getPublicKey().toString();
const item = {
  title: `NEAR testnet ${accountId}`,
  category: 'API_CREDENTIAL',
  fields: [
    { id: 'username', type: 'STRING', label: 'username', value: accountId },
    { id: 'credential', type: 'CONCEALED', label: 'credential', value: kp.toString() },
    { id: 'public_key', type: 'STRING', label: 'public key', value: publicKey },
    { id: 'notesPlain', type: 'STRING', purpose: 'NOTES', label: 'notesPlain', value: 'Kring hackathon demo. NEAR testnet only, no real value. Created via the testnet faucet helper.' },
  ],
};
// op reads the item from a template file. Keep it in RAM-backed /dev/shm, private, and remove it right away.
const dir = mkdtempSync(join(process.env.KRING_TMP || '/dev/shm', 'kring-'));
const tpl = join(dir, 'item.json');
let saved;
try {
  writeFileSync(tpl, JSON.stringify(item), { mode: 0o600 });
  const op = spawnSync('op', ['item', 'create', '--vault', vault, '--template', tpl, '--format', 'json'], { encoding: 'utf8' });
  if (op.status !== 0) throw new Error(`1Password save failed: ${op.stderr.trim()}`);
  saved = JSON.parse(op.stdout);
} finally {
  rmSync(dir, { recursive: true, force: true });
}
// Read it back and compare before the faucet funds anything, so a key can't get lost.
const back = spawnSync('op', ['read', `op://${vault}/${saved.id}/credential`], { encoding: 'utf8' });
if (back.status !== 0 || back.stdout.trim() !== kp.toString()) throw new Error('1Password read-back did not match; not creating the account');
console.log(`Saved and verified key in 1Password item ${saved.id} (vault ${vault})`);

const res = await fetch('https://helper.nearprotocol.com/account', {
  method: 'POST',
  headers: { 'content-type': 'application/json' },
  body: JSON.stringify({ newAccountId: accountId, newAccountPublicKey: publicKey }),
});
const body = await res.text();
if (!res.ok) throw new Error(`Faucet refused (${res.status}): ${body.slice(0, 300)}`);
console.log(`Created ${accountId} with public key ${publicKey}`);
