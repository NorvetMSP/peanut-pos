Security and compliance plan
What you’ll get (at a glance)

JWT end‑to‑end: RS256 with JWKS + key rotation, short‑lived access tokens, refresh tokens, audience & role claims, tenant binding.

Service‑side enforcement: shared auth middleware + RBAC policy helpers; reject non‑admin product creation; consistent X-Tenant-ID ↔ token.tid checks.

Audit Trail: append‑only tables + Kafka audit‑events producer library & a small audit‑service consumer that writes tamper‑evident logs.

PII protection: envelope encryption (AES‑256‑GCM) with per‑tenant DEKs; deterministic hashes for lookup; GDPR export/delete endpoints.

MFA: TOTP enroll/verify flows in Auth; login telemetry + suspicious‑activity flags.

Integration Gateway: API key usage audit, Redis rate‑limit, Coinbase webhook HMAC verify, (optional) partner JWT/OAuth2.

Observability: Prometheus /metrics, structured JSON logs.

DB migrations: Postgres DDL for keys, audit, PII columns, GDPR tombstones.

Phased plan (execute in this order)
Foundations (shared libs)

Add @company/common-auth, @company/common-audit, @company/common-crypto packages.

Wire every microservice to use common auth middleware (JWT + tenant + roles).

Auth Service

Implement JWT minting, JWKS endpoint, key rotation, MFA.

Add API key CRUD (hash at rest) for Integration Gateway partners.

Audit

Publish audit events from all services (including Integration Gateway).

Add audit‑service consumer to persist immutable logs.

Role enforcement

Apply requireRole/requirePermission to protected routes (e.g., Product create).

PII encryption & GDPR

Customer Service: add encrypted columns + read/write hooks.

Add export + delete flows with audit + Kafka events.

Integration Gateway

Add API key usage audit, Redis limiter, Coinbase signature verification.

Optional partner JWT/OAuth2 token issue/validation.

Monitoring

Expose /metrics everywhere; add login/abuse counters; dashboards.

Environment (new/updated)

# Shared
JWT_ISSUER=https://auth.yourdomain.com
JWT_AUDIENCE=pos-api
JWKS_CACHE_TTL_MS=600000

# Auth service
AUTH_DB_URL=postgres://...
ARGON2_MEMORY_COST=15360
ARGON2_TIME_COST=3
ARGON2_PARALLELISM=1
TOKEN_ACCESS_TTL_SECONDS=900
TOKEN_REFRESH_TTL_SECONDS=2592000
MASTER_KEY_HEX=<32-byte hex>     # for envelope encryption of tenant DEKs, api keys
KEY_ROTATION_INTERVAL_HOURS=24

# Kafka
KAFKA_BROKERS=broker1:9092,broker2:9092
KAFKA_CLIENT_ID=pos-platform
AUDIT_TOPIC=audit.events.v1

# Customer service
CUSTOMER_DB_URL=postgres://...

# Integration gateway
REDIS_URL=redis://...
COINBASE_WEBHOOK_SECRET=<from Coinbase>
GATEWAY_RATE_LIMIT_RPM=60

# Audit service
AUDIT_DB_URL=postgres://...


Database migrations (Postgres)

Apply in your migration tool (Knex/Prisma/Flyway/etc.). Names are illustrative.

001_auth_signing_keys.sql

CREATE TABLE auth_signing_keys (
  kid TEXT PRIMARY KEY,
  public_pem TEXT NOT NULL,
  private_pem TEXT NOT NULL,
  alg TEXT NOT NULL DEFAULT 'RS256',
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  rotated_at TIMESTAMPTZ,
  active BOOLEAN NOT NULL DEFAULT TRUE
);

002_auth_refresh_tokens.sql

CREATE TABLE auth_refresh_tokens (
  jti UUID PRIMARY KEY,
  user_id UUID NOT NULL,
  tenant_id TEXT NOT NULL,
  issued_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  expires_at TIMESTAMPTZ NOT NULL,
  revoked_at TIMESTAMPTZ
);

003_auth_api_keys.sql

CREATE TABLE auth_api_keys (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id TEXT NOT NULL,
  name TEXT NOT NULL,
  key_prefix TEXT NOT NULL, -- showable
  key_hash BYTEA NOT NULL,  -- SHA-256(salt||key)
  salt BYTEA NOT NULL,
  created_by UUID NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  revoked_at TIMESTAMPTZ
);

010_audit_logs.sql (for audit‑service)

CREATE TABLE audit_logs (
  id BIGSERIAL PRIMARY KEY,
  occurred_at TIMESTAMPTZ NOT NULL,
  actor_user_id UUID,
  actor_tenant_id TEXT NOT NULL,
  actor_roles TEXT[] NOT NULL,
  action TEXT NOT NULL,
  target_type TEXT,
  target_id TEXT,
  severity TEXT NOT NULL,
  ip INET,
  user_agent TEXT,
  meta JSONB,
  event_id UUID NOT NULL, -- from producer
  prev_row_sig BYTEA,     -- chain-of-custody
  row_sig BYTEA NOT NULL  -- HMAC(master, row)
);
CREATE INDEX ON audit_logs (actor_tenant_id, occurred_at DESC);

020_customer_pii_encryption.sql

ALTER TABLE customers
  ADD COLUMN enc_key_id TEXT, -- which tenant DEK encrypted these fields
  ADD COLUMN email_hash BYTEA, -- deterministic SHA-256(salt||lower(email))
  ADD COLUMN first_name_enc BYTEA,
  ADD COLUMN last_name_enc BYTEA,
  ADD COLUMN email_enc BYTEA,
  ADD COLUMN phone_enc BYTEA;

CREATE INDEX customers_email_hash_idx ON customers (email_hash);

021_tenant_data_keys.sql

CREATE TABLE tenant_data_keys (
  tenant_id TEXT NOT NULL,
  key_id TEXT NOT NULL,
  enc_dek BYTEA NOT NULL,     -- AES-GCM(MASTER_KEY, DEK)
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  rotated_at TIMESTAMPTZ,
  PRIMARY KEY (tenant_id, key_id)
);

030_gdpr_tombstones.sql

CREATE TABLE gdpr_tombstones (
  tenant_id TEXT NOT NULL,
  user_id UUID NOT NULL,
  deleted_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  reason TEXT,
  PRIMARY KEY (tenant_id, user_id)
);

New shared libraries (monorepo packages/)
packages/common-auth/src/index.ts

// Lightweight, framework-agnostic helpers for Express/Fastify.
// pnpm add jose undici
import { createRemoteJWKSet, jwtVerify, JWTPayload } from 'jose';
import { URL } from 'url';

const ISSUER = process.env.JWT_ISSUER!;
const AUDIENCE = process.env.JWT_AUDIENCE!;
const JWKS_URI = new URL('/.well-known/jwks.json', ISSUER).toString();
const JWKS = createRemoteJWKSet(new URL(JWKS_URI));
const JWKS_TTL = Number(process.env.JWKS_CACHE_TTL_MS ?? 600000);

export type AuthContext = {
  sub: string;     // user id
  tid: string;     // tenant id
  roles: string[]; // e.g. ['admin']
  scope?: string[]; // optional scopes
};

export async function verifyBearer(authorization?: string): Promise<AuthContext> {
  if (!authorization?.startsWith('Bearer ')) {
    throw Object.assign(new Error('Missing/invalid Authorization header'), { status: 401 });
  }
  const token = authorization.slice('Bearer '.length);
  const { payload } = await jwtVerify(token, JWKS, {
    issuer: ISSUER,
    audience: AUDIENCE,
    maxTokenAge: '15m',
    clockTolerance: '30s'
  });
  const ctx = mapPayload(payload);
  if (!ctx.tid || !ctx.sub || !Array.isArray(ctx.roles)) {
    throw Object.assign(new Error('Invalid token claims'), { status: 401 });
  }
  return ctx;
}

function mapPayload(p: JWTPayload): AuthContext {
  return {
    sub: String(p.sub),
    tid: String((p as any).tid),
    roles: (p as any).roles ?? [],
    scope: (p as any).scope
  };
}

// Express middleware
export function authMiddleware(req: any, res: any, next: any) {
  verifyBearer(req.headers['authorization'])
    .then(ctx => {
      // Tenant header must match token claim
      const headerTid = req.header('x-tenant-id');
      if (!headerTid || headerTid !== ctx.tid) {
        throw Object.assign(new Error('Tenant mismatch'), { status: 403 });
      }
      (req as any).auth = ctx;
      next();
    })
    .catch(err => res.status(err.status || 401).json({ error: err.message }));
}

export function requireRole(...roles: string[]) {
  return (req: any, res: any, next: any) => {
    const ctx: AuthContext | undefined = (req as any).auth;
    if (!ctx) return res.status(401).json({ error: 'Unauthenticated' });
    if (!roles.some(r => ctx.roles.includes(r))) {
      return res.status(403).json({ error: 'Forbidden: role required', need: roles });
    }
    next();
  };
}

// Permission matrix (extensible)
const PERMISSIONS: Record<string,string[]> = {
  'product:create': ['super_admin','admin'],
  'product:update': ['super_admin','admin'],
  'refund:create' : ['super_admin','admin','manager'],
  'user:create'   : ['super_admin','admin']
};

export function requirePermission(perm: keyof typeof PERMISSIONS) {
  return (req: any, res: any, next: any) => {
    const ctx: AuthContext | undefined = (req as any).auth;
    if (!ctx) return res.status(401).json({ error: 'Unauthenticated' });
    const allowed = PERMISSIONS[perm] ?? [];
    if (!allowed.some(r => ctx.roles.includes(r))) {
      return res.status(403).json({ error: `Forbidden: ${perm} requires ${allowed.join(',')}` });
    }
    next();
  };
}

packages/common-audit/src/index.ts

// pnpm add kafkajs
import { Kafka, logLevel } from 'kafkajs';
import crypto from 'crypto';

const kafka = new Kafka({
  clientId: process.env.KAFKA_CLIENT_ID || 'pos-platform',
  brokers: (process.env.KAFKA_BROKERS || 'localhost:9092').split(','),
  logLevel: logLevel.NOTHING
});
const producer = kafka.producer();
let connected = false;

async function ensure() {
  if (!connected) { await producer.connect(); connected = true; }
}

export type AuditEvent = {
  eventId?: string;
  occurredAt?: string; // ISO
  action: string; // 'product.created', 'login.failed'
  severity: 'info'|'warn'|'high';
  actor: { userId?: string; tenantId: string; roles: string[]; };
  target?: { type?: string; id?: string; };
  ip?: string;
  userAgent?: string;
  meta?: Record<string, any>;
};

export async function audit(ev: AuditEvent) {
  await ensure();
  const eventId = ev.eventId ?? crypto.randomUUID();
  const occurredAt = ev.occurredAt ?? new Date().toISOString();
  const payload = { ...ev, eventId, occurredAt };
  await producer.send({
    topic: process.env.AUDIT_TOPIC || 'audit.events.v1',
    messages: [{ key: ev.actor.tenantId, value: JSON.stringify(payload) }]
  });
}

packages/common-crypto/src/index.ts

// Envelope encryption helpers for per-tenant DEKs.
// pnpm add @noble/hashes
import crypto from 'crypto';
import { sha256 } from '@noble/hashes/sha256';

const MASTER_KEY = Buffer.from(process.env.MASTER_KEY_HEX!, 'hex');

export function generateDEK(): Buffer {
  return crypto.randomBytes(32);
}

export function encryptDEK(dek: Buffer): { iv: Buffer, tag: Buffer, ct: Buffer } {
  const iv = crypto.randomBytes(12);
  const cipher = crypto.createCipheriv('aes-256-gcm', MASTER_KEY, iv);
  const ct = Buffer.concat([cipher.update(dek), cipher.final()]);
  const tag = cipher.getAuthTag();
  return { iv, tag, ct };
}

export function decryptDEK(iv: Buffer, tag: Buffer, ct: Buffer): Buffer {
  const decipher = crypto.createDecipheriv('aes-256-gcm', MASTER_KEY, iv);
  decipher.setAuthTag(tag);
  const pt = Buffer.concat([decipher.update(ct), decipher.final()]);
  return pt;
}

export type Encrypted = { iv: string, tag: string, ct: string }; // base64 strings

export function aesGcmEncrypt(plain: Buffer, dek: Buffer): Encrypted {
  const iv = crypto.randomBytes(12);
  const cipher = crypto.createCipheriv('aes-256-gcm', dek, iv);
  const ct = Buffer.concat([cipher.update(plain), cipher.final()]);
  const tag = cipher.getAuthTag();
  return { iv: iv.toString('base64'), tag: tag.toString('base64'), ct: ct.toString('base64') };
}
export function aesGcmDecrypt(enc: Encrypted, dek: Buffer): Buffer {
  const iv = Buffer.from(enc.iv, 'base64');
  const tag = Buffer.from(enc.tag, 'base64');
  const ct  = Buffer.from(enc.ct,  'base64');
  const decipher = crypto.createDecipheriv('aes-256-gcm', dek, iv);
  decipher.setAuthTag(tag);
  return Buffer.concat([decipher.update(ct), decipher.final()]);
}

// Deterministic hash for lookups (email) using SHA-256(salt||lower(value))
export function stableHash(value: string, salt: Buffer): Buffer {
  const lower = value.trim().toLowerCase();
  const h = sha256.create();
  h.update(salt);
  h.update(Buffer.from(lower));
  return Buffer.from(h.digest());
}

Auth Service (new/updated files)

Key features

RS256 JWT with kid header, JWKS endpoint

Short‑lived Access, long‑lived Refresh tokens

MFA TOTP enroll/verify

API Keys (hashed) for partners

Login telemetry + audit

Dependencies: pnpm add jose argon2 otplib qrcode kafkajs pg (or your ORM)

auth-service/src/jwt.ts

import crypto from 'crypto';
import { SignJWT, importPKCS8, exportJWK } from 'jose';
import { Pool } from 'pg';

const pool = new Pool({ connectionString: process.env.AUTH_DB_URL });

export async function currentSigningKey() {
  const { rows } = await pool.query(
    'SELECT kid, private_pem, public_pem FROM auth_signing_keys WHERE active = TRUE ORDER BY created_at DESC LIMIT 1'
  );
  if (!rows[0]) throw new Error('No active signing key');
  return rows[0] as { kid: string, private_pem: string, public_pem: string };
}

export async function mintAccessToken(payload: {
  sub: string; tid: string; roles: string[]; scope?: string[];
}) {
  const key = await currentSigningKey();
  const alg = 'RS256';
  const pk = await importPKCS8(key.private_pem, alg);
  const now = Math.floor(Date.now()/1000);
  const exp = now + Number(process.env.TOKEN_ACCESS_TTL_SECONDS || 900);
  return new SignJWT({ ...payload })
    .setProtectedHeader({ alg, kid: key.kid })
    .setIssuer(process.env.JWT_ISSUER!)
    .setAudience(process.env.JWT_AUDIENCE!)
    .setSubject(payload.sub)
    .setIssuedAt(now)
    .setExpirationTime(exp)
    .sign(pk);
}

export async function mintRefreshToken(userId: string, tenantId: string) {
  const jti = crypto.randomUUID();
  const now = new Date();
  const ttlSec = Number(process.env.TOKEN_REFRESH_TTL_SECONDS || 2592000);
  const exp = new Date(now.getTime() + ttlSec*1000);
  await pool.query(
    'INSERT INTO auth_refresh_tokens (jti, user_id, tenant_id, issued_at, expires_at) VALUES ($1,$2,$3,$4,$5)',
    [jti, userId, tenantId, now, exp]
  );
  return { refreshToken: jti, expiresAt: exp };
}

export async function exposeJWKS() {
  const { rows } = await pool.query('SELECT kid, public_pem FROM auth_signing_keys WHERE active = TRUE');
  const keys = await Promise.all(rows.map(async (r:any) => {
    const jwk = await exportJWK({ type: 'public', format: 'pem', key: r.public_pem } as any);
    // jose exportJWK for PEM isn't direct; alternative: parse to KeyObject then exportJWK
    // Simpler: build JWK via crypto
    const keyObj = crypto.createPublicKey(r.public_pem);
    const jwk2 = await exportJWK(keyObj as any);
    return { ...jwk2, kid: r.kid, alg: 'RS256', use: 'sig' };
  }));
  return { keys };
}


auth-service/src/routes/jwks.ts

import { Router } from 'express';
import { exposeJWKS } from '../jwt';

export const jwksRouter = Router();
jwksRouter.get('/.well-known/jwks.json', async (req, res) => {
  try { res.json(await exposeJWKS()); }
  catch (e:any) { res.status(500).json({ error: e.message }); }
});


auth-service/src/routes/login.ts

import { Router } from 'express';
import argon2 from 'argon2';
import { mintAccessToken, mintRefreshToken } from '../jwt';
import { audit } from '@company/common-audit';
import { Pool } from 'pg';
import { authenticator } from 'otplib';

const pool = new Pool({ connectionString: process.env.AUTH_DB_URL });
export const loginRouter = Router();

loginRouter.post('/v1/auth/login', async (req, res) => {
  const { email, password, tenantId, mfaCode } = req.body ?? {};
  const ip = req.ip; const ua = String(req.headers['user-agent'] || '');
  try {
    const { rows } = await pool.query(
      'SELECT id, tenant_id, pass_hash, roles, mfa_secret FROM users WHERE lower(email)=lower($1) AND tenant_id=$2',
      [email, tenantId]
    );
    if (!rows[0]) {
      await audit({ action:'login.failed', severity:'warn', actor:{tenantId, roles:[], userId:undefined}, ip, userAgent:ua, meta:{email} });
      return res.status(401).json({ error: 'Invalid credentials' });
    }
    const u = rows[0];
    const ok = await argon2.verify(u.pass_hash, password);
    if (!ok) {
      await audit({ action:'login.failed', severity:'warn', actor:{tenantId, roles:u.roles, userId:u.id}, ip, userAgent:ua });
      return res.status(401).json({ error: 'Invalid credentials' });
    }
    if (u.mfa_secret) {
      if (!mfaCode || !authenticator.verify({ token: mfaCode, secret: u.mfa_secret })) {
        return res.status(401).json({ error: 'MFA required/invalid' });
      }
    }
    const token = await mintAccessToken({ sub: u.id, tid: u.tenant_id, roles: u.roles });
    const { refreshToken, expiresAt } = await mintRefreshToken(u.id, u.tenant_id);
    await audit({ action:'login.success', severity:'info', actor:{tenantId: u.tenant_id, roles:u.roles, userId:u.id}, ip, userAgent:ua });
    res.json({ access_token: token, token_type:'Bearer', expires_in: Number(process.env.TOKEN_ACCESS_TTL_SECONDS||900), refresh_token: refreshToken, refresh_expires_at: expiresAt });
  } catch (e:any) {
    res.status(500).json({ error: e.message });
  }
});


auth-service/src/routes/mfa.ts

import { Router } from 'express';
import { authenticator } from 'otplib';
import QRCode from 'qrcode';
import { Pool } from 'pg';
import { authMiddleware } from '@company/common-auth';

const pool = new Pool({ connectionString: process.env.AUTH_DB_URL });
export const mfaRouter = Router();

mfaRouter.post('/v1/auth/mfa/enroll', authMiddleware, async (req:any, res) => {
  const { sub, tid } = req.auth;
  const secret = authenticator.generateSecret();
  const otpauth = authenticator.keyuri(sub, `POS(${tid})`, secret);
  const png = await QRCode.toDataURL(otpauth);
  await pool.query('UPDATE users SET mfa_secret=$1 WHERE id=$2 AND tenant_id=$3', [secret, sub, tid]);
  res.json({ qr_data_url: png, secret });
});

mfaRouter.post('/v1/auth/mfa/verify', authMiddleware, async (req:any, res) => {
  const { sub, tid } = req.auth;
  const { token } = req.body ?? {};
  const { rows } = await pool.query('SELECT mfa_secret FROM users WHERE id=$1 AND tenant_id=$2', [sub, tid]);
  if (!rows[0]?.mfa_secret) return res.status(400).json({ error: 'Not enrolled' });
  const ok = authenticator.verify({ token, secret: rows[0].mfa_secret });
  res.json({ verified: !!ok });
});


auth-service/src/routes/apiKeys.ts

import { Router } from 'express';
import crypto from 'crypto';
import { Pool } from 'pg';
import { authMiddleware, requireRole } from '@company/common-auth';

const pool = new Pool({ connectionString: process.env.AUTH_DB_URL });
export const apiKeysRouter = Router();

function hashKey(key:string, salt:Buffer) {
  const h = crypto.createHash('sha256');
  h.update(salt); h.update(key);
  return h.digest();
}

apiKeysRouter.post('/v1/partners/apikeys', authMiddleware, requireRole('super_admin','admin'), async (req:any,res) => {
  const { name } = req.body ?? {};
  const raw = 'pk_' + crypto.randomBytes(24).toString('base64url');
  const prefix = raw.slice(0,8);
  const salt = crypto.randomBytes(16);
  const keyHash = hashKey(raw, salt);
  await pool.query('INSERT INTO auth_api_keys (tenant_id, name, key_prefix, key_hash, salt, created_by) VALUES ($1,$2,$3,$4,$5,$6)',
    [req.auth.tid, name || prefix, prefix, keyHash, salt, req.auth.sub]);
  res.json({ api_key: raw, prefix });
});

apiKeysRouter.post('/v1/partners/apikeys/revoke', authMiddleware, requireRole('super_admin','admin'), async (req:any,res) => {
  const { id } = req.body ?? {};
  await pool.query('UPDATE auth_api_keys SET revoked_at=now() WHERE id=$1 AND tenant_id=$2', [id, req.auth.tid]);
  res.json({ ok: true });
});


auth-service/src/routes/refresh.ts

import { Router } from 'express';
import { Pool } from 'pg';
import { mintAccessToken } from '../jwt';

const pool = new Pool({ connectionString: process.env.AUTH_DB_URL });
export const refreshRouter = Router();

refreshRouter.post('/v1/auth/refresh', async (req, res) => {
  const { refresh_token, tenantId } = req.body ?? {};
  const { rows } = await pool.query(
    'SELECT r.user_id, r.tenant_id, u.roles, r.expires_at FROM auth_refresh_tokens r JOIN users u ON u.id=r.user_id WHERE jti=$1 AND r.tenant_id=$2 AND r.revoked_at IS NULL',
    [refresh_token, tenantId]
  );
  const r = rows[0];
  if (!r || new Date(r.expires_at) < new Date()) return res.status(401).json({ error: 'Invalid refresh' });
  const access = await mintAccessToken({ sub: r.user_id, tid: r.tenant_id, roles: r.roles });
  res.json({ access_token: access, token_type:'Bearer', expires_in: Number(process.env.TOKEN_ACCESS_TTL_SECONDS||900) });
});

auth-service/src/jobs/rotate-keys.ts

import crypto from 'crypto';
import { Pool } from 'pg';

const pool = new Pool({ connectionString: process.env.AUTH_DB_URL });

export async function rotateKeys() {
  const { privateKey, publicKey } = crypto.generateKeyPairSync('rsa', { modulusLength: 2048 });
  const private_pem = privateKey.export({ type:'pkcs8', format:'pem' }).toString();
  const public_pem  = publicKey.export({ type:'spki',  format:'pem' }).toString();
  const kid = crypto.randomUUID();
  await pool.query('UPDATE auth_signing_keys SET active=FALSE WHERE active=TRUE');
  await pool.query('INSERT INTO auth_signing_keys (kid, private_pem, public_pem, active) VALUES ($1,$2,$3,TRUE)', [kid, private_pem, public_pem]);
}

// run on interval via your scheduler or process manager
if (require.main === module) {
  rotateKeys().then(()=>process.exit(0)).catch(e=>{console.error(e);process.exit(1);});
}


Product Service — enforce RBAC (diff)

diff --git a/product-service/src/server.ts b/product-service/src/server.ts
@@
-import express from 'express';
+import express from 'express';
+import { authMiddleware, requirePermission } from '@company/common-auth';
+import { audit } from '@company/common-audit';

 const app = express();
 app.use(express.json());
+app.use(authMiddleware);

-app.post('/v1/products', async (req, res) => {
+app.post('/v1/products', requirePermission('product:create'), async (req:any, res) => {
   const tenantId = req.header('x-tenant-id');
   const body = req.body;
   // ... create product in DB, scoped to tenantId
+  await audit({
+    action: 'product.created',
+    severity: 'info',
+    actor: { tenantId: req.auth.tid, roles: req.auth.roles, userId: req.auth.sub },
+    target: { type: 'product', id: '...db-id...' },
+    meta: { sku: body.sku }
+  });
   res.status(201).json({ ok:true });
 });

Customer Service — PII encryption + GDPR

Repository hooks (write‑encrypt / read‑decrypt)

customer-service/src/crypto/tenantKeys.ts

import { Pool } from 'pg';
import { decryptDEK, encryptDEK, generateDEK } from '@company/common-crypto';

const pool = new Pool({ connectionString: process.env.CUSTOMER_DB_URL });

export async function getTenantDEK(tenantId: string) {
  const { rows } = await pool.query('SELECT key_id, enc_dek FROM tenant_data_keys WHERE tenant_id=$1 ORDER BY created_at DESC LIMIT 1', [tenantId]);
  if (rows[0]) {
    const buf = Buffer.from(rows[0].enc_dek, 'base64');
    const iv = buf.subarray(0,12), tag = buf.subarray(12,28), ct = buf.subarray(28);
    const dek = decryptDEK(iv, tag, ct);
    return { keyId: rows[0].key_id, dek };
  }
  const dek = generateDEK();
  const { iv, tag, ct } = encryptDEK(dek);
  const keyId = `dek_${Date.now()}`;
  const packed = Buffer.concat([iv, tag, ct]).toString('base64');
  await pool.query('INSERT INTO tenant_data_keys (tenant_id, key_id, enc_dek) VALUES ($1,$2,$3)', [tenantId, keyId, packed]);
  return { keyId, dek };
}


customer-service/src/repository/customers.ts

import { Pool } from 'pg';
import { aesGcmEncrypt, aesGcmDecrypt, stableHash } from '@company/common-crypto';
import { getTenantDEK } from '../crypto/tenantKeys';
import crypto from 'crypto';

const pool = new Pool({ connectionString: process.env.CUSTOMER_DB_URL });
const EMAIL_SALT = Buffer.from(process.env.MASTER_KEY_HEX!, 'hex').subarray(0,16); // reuse derivation or separate env

type CustomerInput = { firstName: string; lastName: string; email: string; phone?: string; };

export async function createCustomer(tenantId: string, input: CustomerInput) {
  const { keyId, dek } = await getTenantDEK(tenantId);
  const emailHash = stableHash(input.email, EMAIL_SALT);
  const firstEnc = aesGcmEncrypt(Buffer.from(input.firstName), dek);
  const lastEnc  = aesGcmEncrypt(Buffer.from(input.lastName), dek);
  const emailEnc = aesGcmEncrypt(Buffer.from(input.email), dek);
  const phoneEnc = input.phone ? aesGcmEncrypt(Buffer.from(input.phone), dek) : null;

  const { rows } = await pool.query(
    `INSERT INTO customers (tenant_id, enc_key_id, email_hash, first_name_enc, last_name_enc, email_enc, phone_enc)
     VALUES ($1,$2,$3,$4,$5,$6,$7)
     RETURNING id`,
    [
      tenantId, keyId, emailHash,
      Buffer.from(JSON.stringify(firstEnc)), Buffer.from(JSON.stringify(lastEnc)),
      Buffer.from(JSON.stringify(emailEnc)), phoneEnc ? Buffer.from(JSON.stringify(phoneEnc)) : null
    ]
  );
  return rows[0].id;
}

export async function getByEmail(tenantId: string, email: string) {
  const { keyId, dek } = await getTenantDEK(tenantId);
  const emailHash = stableHash(email, EMAIL_SALT);
  const { rows } = await pool.query('SELECT id, first_name_enc, last_name_enc, email_enc, phone_enc FROM customers WHERE tenant_id=$1 AND email_hash=$2 LIMIT 1', [tenantId, emailHash]);
  if (!rows[0]) return null;
  const dec = (buf: Buffer | null) => {
    if (!buf) return null;
    const enc = JSON.parse(buf.toString());
    return aesGcmDecrypt(enc, dek).toString('utf8');
  };
  return {
    id: rows[0].id,
    firstName: dec(rows[0].first_name_enc),
    lastName: dec(rows[0].last_name_enc),
    email: dec(rows[0].email_enc),
    phone: dec(rows[0].phone_enc)
  };
}


GDPR endpoints — customer-service/src/routes/gdpr.ts

import { Router } from 'express';
import { authMiddleware, requireRole } from '@company/common-auth';
import { audit } from '@company/common-audit';
import { getByEmail } from '../repository/customers';
import { Pool } from 'pg';

const pool = new Pool({ connectionString: process.env.CUSTOMER_DB_URL });
export const gdprRouter = Router();
gdprRouter.use(authMiddleware);

// Export by email (admin/manager)
gdprRouter.post('/v1/gdpr/export', requireRole('super_admin','admin','manager'), async (req:any,res) => {
  const { email } = req.body ?? {};
  const data = await getByEmail(req.auth.tid, email);
  await audit({ action:'gdpr.export', severity:'info', actor:{tenantId:req.auth.tid, roles:req.auth.roles, userId:req.auth.sub}, meta:{ email } });
  res.json({ data });
});

// Delete user account (admin)
gdprRouter.post('/v1/gdpr/delete', requireRole('super_admin','admin'), async (req:any,res) => {
  const { userId } = req.body ?? {};
  // Soft-delete + scramble PII; keep minimal order links if needed for financial retention policies
  await pool.query('UPDATE customers SET first_name_enc=NULL, last_name_enc=NULL, email_enc=NULL, phone_enc=NULL WHERE tenant_id=$1 AND id=$2', [req.auth.tid, userId]);
  await pool.query('INSERT INTO gdpr_tombstones (tenant_id, user_id, reason) VALUES ($1,$2,$3) ON CONFLICT DO NOTHING', [req.auth.tid, userId, 'right-to-be-forgotten']);
  await audit({ action:'gdpr.delete', severity:'high', actor:{tenantId:req.auth.tid, roles:req.auth.roles, userId:req.auth.sub}, target:{type:'customer', id:userId} });
  res.json({ ok:true });
});


Integration Gateway

API key auth + audit + Redis rate limiter + Coinbase verify

integration-gateway/src/middleware/apiKeyAuth.ts

import { Pool } from 'pg';
import crypto from 'crypto';
import { audit } from '@company/common-audit';

const pool = new Pool({ connectionString: process.env.AUTH_DB_URL });

function hashKey(key:string, salt:Buffer) {
  return crypto.createHash('sha256').update(salt).update(key).digest();
}

export async function apiKeyAuth(req:any,res:any,next:any) {
  const key = req.header('x-api-key');
  const tenantId = req.header('x-tenant-id');
  if (!key || !tenantId) return res.status(401).json({ error: 'Missing key/tenant' });

  const prefix = key.slice(0,8);
  const { rows } = await pool.query('SELECT id, key_hash, salt, revoked_at FROM auth_api_keys WHERE tenant_id=$1 AND key_prefix=$2', [tenantId, prefix]);
  if (!rows[0] || rows[0].revoked_at) return res.status(401).json({ error:'Invalid key' });
  const ok = crypto.timingSafeEqual(rows[0].key_hash, hashKey(key, rows[0].salt));
  if (!ok) return res.status(401).json({ error:'Invalid key' });

  req.partner = { tenantId, keyId: rows[0].id, prefix };

  await audit({
    action: 'apikey.used',
    severity: 'info',
    actor: { tenantId, roles:['partner'], userId: undefined },
    meta: { route: req.originalUrl, method: req.method, keyPrefix: prefix }
  });

  next();
}

integration-gateway/src/middleware/rateLimiter.ts

import { createClient } from 'redis';

const client = createClient({ url: process.env.REDIS_URL });
client.connect();

const RPM = Number(process.env.GATEWAY_RATE_LIMIT_RPM || 60);

export async function rateLimiter(req:any,res:any,next:any) {
  const key = `rl:${req.partner?.tenantId || req.ip}`;
  const now = Math.floor(Date.now() / 1000);
  const window = Math.floor(now / 60);
  const redisKey = `${key}:${window}`;
  const cnt = await client.incr(redisKey);
  if (cnt === 1) await client.expire(redisKey, 60);
  if (cnt > RPM) return res.status(429).json({ error: 'Rate limit exceeded' });
  next();
}

integration-gateway/src/routes/webhooks/coinbase.ts

import { Router } from 'express';
import crypto from 'crypto';
import { audit } from '@company/common-audit';

const SECRET = process.env.COINBASE_WEBHOOK_SECRET!;
export const coinbaseRouter = Router();

function verify(bodyRaw: string, signature: string) {
  const h = crypto.createHmac('sha256', SECRET);
  h.update(bodyRaw);
  const expected = h.digest('hex');
  return crypto.timingSafeEqual(Buffer.from(signature, 'hex'), Buffer.from(expected, 'hex'));
}

coinbaseRouter.post('/webhooks/coinbase', express.raw({ type: '*/*' }), async (req:any,res) => {
  const sig = req.header('X-CC-Webhook-Signature') || '';
  const ok = verify(req.body.toString('utf8'), sig);
  if (!ok) return res.status(400).json({ error:'Invalid signature' });

  const event = JSON.parse(req.body.toString('utf8'));
  // ... process event safely
  await audit({ action:'webhook.coinbase', severity:'info', actor:{tenantId: event?.data?.metadata?.tenantId ?? 'unknown', roles:['webhook']}, meta:{ id: event.id, type: event.type } });

  res.json({ received: true });
});


Wire the middlewares on partner routes:

diff --git a/integration-gateway/src/server.ts b/integration-gateway/src/server.ts
@@
-import express from 'express';
+import express from 'express';
 import cors from 'cors';
 import helmet from 'helmet';
+import { apiKeyAuth } from './middleware/apiKeyAuth';
+import { rateLimiter } from './middleware/rateLimiter';
+import { coinbaseRouter } from './routes/webhooks/coinbase';

 const app = express();
 app.use(helmet());
 app.use(cors({ origin: [/pos\.yourdomain\.com$/, /admin\.yourdomain\.com$/], credentials: true }));
 app.use(express.json());

-// Partner routes:
-app.post('/v1/ext/orders', ..., handler);
+app.use('/v1/ext', apiKeyAuth, rateLimiter);
+app.post('/v1/ext/orders', /* handler */ (req,res)=>res.json({ok:true}));

+// Webhooks
+app.use(coinbaseRouter);

 app.get('/health', (req,res)=>res.json({ok:true}));


Audit Service (consumer)

import { Kafka } from 'kafkajs';
import { Pool } from 'pg';
import crypto from 'crypto';

const kafka = new Kafka({ clientId: process.env.KAFKA_CLIENT_ID || 'audit', brokers: (process.env.KAFKA_BROKERS || '').split(',') });
const consumer = kafka.consumer({ groupId: 'audit-service-v1' });
const pool = new Pool({ connectionString: process.env.AUDIT_DB_URL });
const MASTER = Buffer.from(process.env.MASTER_KEY_HEX!, 'hex');

function signRow(row:any) {
  const h = crypto.createHmac('sha256', MASTER);
  const s = JSON.stringify({ occurred_at: row.occurred_at, actor_tenant_id: row.actor_tenant_id, action: row.action, target_type: row.target_type, target_id: row.target_id, meta: row.meta });
  h.update(s);
  return h.digest();
}

async function getPrevSig(tenantId: string) {
  const { rows } = await pool.query('SELECT row_sig FROM audit_logs WHERE actor_tenant_id=$1 ORDER BY occurred_at DESC, id DESC LIMIT 1', [tenantId]);
  return rows[0]?.row_sig || null;
}

async function run() {
  await consumer.connect();
  await consumer.subscribe({ topic: process.env.AUDIT_TOPIC || 'audit.events.v1' });
  await consumer.run({
    eachMessage: async ({ message }) => {
      const ev = JSON.parse(String(message.value));
      const prev = await getPrevSig(ev.actor.tenantId);
      const res = await pool.query(
        `INSERT INTO audit_logs
        (occurred_at, actor_user_id, actor_tenant_id, actor_roles, action, target_type, target_id, severity, ip, user_agent, meta, event_id, prev_row_sig, row_sig)
        VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)
        RETURNING id, occurred_at, actor_tenant_id, action, target_type, target_id, meta`,
        [ev.occurredAt, ev.actor.userId || null, ev.actor.tenantId, ev.actor.roles, ev.action, ev.target?.type || null, ev.target?.id || null, ev.severity, ev.ip || null, ev.userAgent || null, ev.meta || {}, ev.eventId, prev, Buffer.alloc(0)]
      );
      const row = res.rows[0];
      const sig = signRow(row);
      await pool.query('UPDATE audit_logs SET row_sig=$1 WHERE id=$2', [sig, row.id]);
    }
  });
}
run().catch(e => { console.error(e); process.exit(1); });


Observability (add to each service)
<service>/src/metrics.ts

import client from 'prom-client';
const registry = new client.Registry();
client.collectDefaultMetrics({ register: registry });
export const requestCounter = new client.Counter({ name:'http_requests_total', help:'count', labelNames:['route','method','status'] });
registry.registerMetric(requestCounter);
export function metricsHandler(req:any,res:any){ res.set('Content-Type', registry.contentType); registry.metrics().then(m=>res.send(m)); }


Wire into server:

diff --git a/<service>/src/server.ts b/<service>/src/server.ts
@@
+import { metricsHandler, requestCounter } from './metrics';
+app.use((req,res,next)=>{ const end = res.end; res.end = function(...args:any){ requestCounter.inc({ route:req.path, method:req.method, status: res.statusCode }); return end.apply(this,args); }; next(); });
+app.get('/metrics', metricsHandler);


Tests (illustrative)

product-service/test/authz.spec.ts

import request from 'supertest';
import { mintAccessToken } from '../../auth-service/src/jwt';

it('rejects product create for cashier', async () => {
  const token = await mintAccessToken({ sub:'u1', tid:'t1', roles:['cashier'] });
  await request(app)
    .post('/v1/products')
    .set('Authorization', `Bearer ${token}`)
    .set('X-Tenant-ID','t1')
    .send({ name:'Test' })
    .expect(403);
});


Rollout guide (no downtime)

Deploy shared libs and add as dependency in each service.

Auth Service

Run migrations 001–003.

Generate first signing key (rotate-keys.ts once).

Deploy JWKS + login/refresh/MFA/api‑keys routes.

Service adoption

Add authMiddleware & role checks to write endpoints (start with highest‑risk: refunds, user mgmt, product create/update).

Keep old UUID token verification in gateway temporarily if you must bridge; but disable after clients move to JWT.

Audit

Deploy audit‑service and direct all services to emit audit events via @company/common-audit.

Customer PII

Run migrations 020–021.

Deploy encryption hooks; backfill existing rows by rewriting PII columns through the repository or one‑time script.

GDPR

Deploy routes; wire Admin UI to trigger export/delete.

Integration Gateway

Deploy API key audit + rate limiter; deploy Coinbase verify with secret; test with Coinbase webhook test utility.

Metrics + Alerts

Scrape /metrics; create dashboards & alerts for 401/403 spikes, rate limit hits, login.failed.

PCI & Compliance checklists (skeleton docs to add to your repo)

/docs/security/README.md

/docs/security/jwt-architecture.md

/docs/security/audit-model.md

/docs/security/pii-encryption.md

/docs/compliance/pci-dss-attestation-scope.md

/docs/compliance/gdpr-processes.md

Each doc should cover: data flows, components in scope/out, key handling, retention, incident response, and how the integration with external payment providers keeps cardholder data out of your environment (scope reduction).

Threat modeling (quick highlights)

Token replay / header spoofing: Mitigated by server‑side JWT verify, aud/iss checks, X‑Tenant‑ID⇔tid match.

Privilege escalation: Mitigated by backend RBAC checks (no frontend trust).

Webhook forgery: Mitigated by HMAC verification and (optional) timestamp tolerance.

PII exposure at rest: Mitigated by per‑tenant DEKs + AES‑GCM, hashed lookup for emails.

Audit tampering: Mitigated by append‑only table + chained HMAC signatures; Kafka provides durability.

What to change in your code today

Add the three shared packages and import them across services.

Swap any UUID token checks for authMiddleware.

Gate sensitive routes with requireRole/requirePermission.

Emit audit(...) events on sensitive actions.

Merge the PII repo changes into Customer Service and run migrations.

Integrate Integration Gateway middlewares for API key usage + Redis limiter + Coinbase verify.