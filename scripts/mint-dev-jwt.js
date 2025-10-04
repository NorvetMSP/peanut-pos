// Simple RS256 dev JWT minter. Requires jwt-dev.pem at repo root.
// Usage:
//   npm run mint:jwt -- --tenant <uuid> --roles Admin,Cashier --iss https://auth.novapos.local --aud novapos-frontend
// Outputs a JWT string to stdout.

const fs = require('fs');
const path = require('path');
const jwt = require('jsonwebtoken');

function arg(name, def) {
  const idx = process.argv.indexOf(`--${name}`);
  if (idx !== -1 && process.argv[idx + 1] && !process.argv[idx + 1].startsWith('--')) {
    return process.argv[idx + 1];
  }
  return def;
}

function bool(name, def=false) {
  const idx = process.argv.indexOf(`--${name}`);
  if (idx !== -1) return true;
  return def;
}

const repoRoot = process.cwd();
const pemPath = path.join(repoRoot, 'jwt-dev.pem');
if (!fs.existsSync(pemPath)) {
  console.error(`Missing jwt-dev.pem at ${pemPath}`);
  process.exit(1);
}
const privateKey = fs.readFileSync(pemPath, 'utf8');

const tenant = arg('tenant');
if (!tenant) {
  console.error('Missing --tenant <uuid>');
  process.exit(1);
}
const rolesArg = arg('roles', 'Admin');
const roles = rolesArg.split(',').map(r => r.trim()).filter(Boolean);
const iss = arg('iss', 'https://auth.novapos.local');
const audArg = arg('aud', 'novapos-frontend,novapos-admin');
const audMode = arg('audMode', 'list'); // 'single' | 'list'
const sub = arg('sub', 'dev-user');
const expMins = parseInt(arg('expMins', '30'), 10);
const kid = arg('kid');

const now = Math.floor(Date.now() / 1000);
const payload = {
  sub,
  iss,
  iat: now,
  exp: now + expMins * 60,
  tenant_id: tenant,
  roles,
};
if (audMode === 'single') {
  payload.aud = audArg; // exact string
} else {
  payload.aud = audArg.split(',').map(a => a.trim()).filter(Boolean);
}
const signOpts = { algorithm: 'RS256' };
if (kid) signOpts.keyid = kid;

try {
  const token = jwt.sign(payload, privateKey, signOpts);
  process.stdout.write(token);
} catch (e) {
  console.error('Failed to sign JWT:', e.message);
  process.exit(1);
}
