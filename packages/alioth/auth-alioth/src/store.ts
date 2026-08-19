/**
 * Auth storage: `dsh_alioth_auth` schema — deliberately SEPARATE from the
 * registry schemas (`isahl_meta`, `dsh_alioth`): `resetRegistry()` drops those
 * and re-bootstraps from the model baseline, but user/session data must
 * survive resets. Bootstrap is idempotent (CREATE IF NOT EXISTS).
 * @module @dsh-alioth/auth-alioth/store
 */

import type { Context } from '@deepseek-ai/cordis'

export interface UserRow {
  readonly id: string
  readonly username: string
  readonly passwordHash: string
  readonly namespace: string
  readonly role: 'admin' | 'user'
  readonly createdAt: string
}

export interface SessionRow {
  readonly tokenHash: string
  readonly userId: string
  readonly sessionId: string | null
  readonly expiresAt: string
}

export const AUTH_SCHEMA = 'dsh_alioth_auth'

/** Idempotent bootstrap: schema + tables + indexes (never touches registry). */
export async function ensureAuthSchema(ctx: Context): Promise<void> {
  await ctx.aliothEnv.sql(`
    CREATE SCHEMA IF NOT EXISTS ${AUTH_SCHEMA};
    CREATE TABLE IF NOT EXISTS ${AUTH_SCHEMA}.users (
      id text PRIMARY KEY,
      username text NOT NULL UNIQUE,
      password_hash text NOT NULL,
      namespace text NOT NULL UNIQUE,
      role text NOT NULL DEFAULT 'user' CHECK (role IN ('admin', 'user')),
      created_at timestamptz NOT NULL DEFAULT now()
    );
    CREATE TABLE IF NOT EXISTS ${AUTH_SCHEMA}.sessions (
      token_hash text PRIMARY KEY,
      user_id text NOT NULL REFERENCES ${AUTH_SCHEMA}.users(id) ON DELETE CASCADE,
      session_id text,
      expires_at timestamptz NOT NULL
    );
    CREATE INDEX IF NOT EXISTS sessions_user_idx ON ${AUTH_SCHEMA}.sessions (user_id);
  `)
}

export async function insertUser(
  ctx: Context,
  user: { readonly id: string; readonly username: string; readonly passwordHash: string; readonly namespace: string; readonly role: 'admin' | 'user' },
): Promise<void> {
  await ctx.aliothEnv.sql(
    `INSERT INTO ${AUTH_SCHEMA}.users (id, username, password_hash, namespace, role)
     VALUES ($1, $2, $3, $4, $5)`,
    [user.id, user.username, user.passwordHash, user.namespace, user.role],
  )
}

export async function userByUsername(ctx: Context, username: string): Promise<UserRow | null> {
  const result = await ctx.aliothEnv.sql<UserRow>(
    `SELECT id, username, password_hash AS "passwordHash", namespace, role, created_at AS "createdAt"
     FROM ${AUTH_SCHEMA}.users WHERE username = $1`,
    [username],
  )
  return result.rows[0] ?? null
}

export async function userById(ctx: Context, id: string): Promise<UserRow | null> {
  const result = await ctx.aliothEnv.sql<UserRow>(
    `SELECT id, username, password_hash AS "passwordHash", namespace, role, created_at AS "createdAt"
     FROM ${AUTH_SCHEMA}.users WHERE id = $1`,
    [id],
  )
  return result.rows[0] ?? null
}

export async function userByNamespace(ctx: Context, namespace: string): Promise<UserRow | null> {
  const result = await ctx.aliothEnv.sql<UserRow>(
    `SELECT id, username, password_hash AS "passwordHash", namespace, role, created_at AS "createdAt"
     FROM ${AUTH_SCHEMA}.users WHERE namespace = $1`,
    [namespace],
  )
  return result.rows[0] ?? null
}

export async function insertSession(
  ctx: Context,
  session: { readonly tokenHash: string; readonly userId: string; readonly sessionId: string | null; readonly expiresAt: Date },
): Promise<void> {
  await ctx.aliothEnv.sql(
    `INSERT INTO ${AUTH_SCHEMA}.sessions (token_hash, user_id, session_id, expires_at)
     VALUES ($1, $2, $3, $4)`,
    [session.tokenHash, session.userId, session.sessionId, session.expiresAt.toISOString()],
  )
}

export async function sessionByTokenHash(ctx: Context, tokenHash: string): Promise<SessionRow | null> {
  const result = await ctx.aliothEnv.sql<SessionRow>(
    `SELECT token_hash AS "tokenHash", user_id AS "userId", session_id AS "sessionId", expires_at AS "expiresAt"
     FROM ${AUTH_SCHEMA}.sessions WHERE token_hash = $1`,
    [tokenHash],
  )
  return result.rows[0] ?? null
}

export async function bindSession(ctx: Context, tokenHash: string, sessionId: string): Promise<void> {
  await ctx.aliothEnv.sql(
    `UPDATE ${AUTH_SCHEMA}.sessions SET session_id = $2 WHERE token_hash = $1`,
    [tokenHash, sessionId],
  )
}

export async function deleteSession(ctx: Context, tokenHash: string): Promise<void> {
  await ctx.aliothEnv.sql(`DELETE FROM ${AUTH_SCHEMA}.sessions WHERE token_hash = $1`, [tokenHash])
}

export async function deleteExpiredSessions(ctx: Context): Promise<void> {
  await ctx.aliothEnv.sql(`DELETE FROM ${AUTH_SCHEMA}.sessions WHERE expires_at < now()`)
}
