/**
 * PostgreSQL lifecycle for the Alioth environment.
 *
 * One path only: an externally provisioned server reached through a URL
 * (`ALIOTH_DATABASE_URL` / `Config.databaseUrl`) — the deployment stack's own
 * PostgreSQL 18 (dev/m2/prod: host PG 18.6; container: the PGDG build started by
 * `scripts/docker-entry.sh`). The plugin NEVER provisions a database: a silently
 * auto-started cluster competes with the environment's server for the data and
 * hides a missing DSN until something else fails.
 * @module @dsh-alioth/env-alioth/pg
 */

import { Client, type QueryResult, type QueryResultRow } from 'pg'

/**
 * The one way to run SQL against the resolved database.
 *
 * Every caller funnels through the handle instead of holding a `Client`: a session can
 * outlive its socket (server restart, idle kill, network drop), and a bare client never
 * recovers — every later query then fails with pg's "not queryable" guard. The handle
 * reconnects once for exactly those failures.
 */
export type QueryFn = <T extends QueryResultRow>(
  text: string,
  values?: readonly unknown[],
) => Promise<QueryResult<T>>

/** A live query surface plus the URL it came from. */
export interface PgHandle {
  readonly query: QueryFn
  /** Connection URL (contains credentials — mask before display). */
  readonly url: string
  /** Close the connection. */
  close(): Promise<void>
}

/**
 * The failure where pg *refused to send* the statement because the connection was
 * already known dead — so it provably never executed server-side, and replaying it
 * on a fresh connection cannot double-apply a write.
 */
function isUnsentConnectionFailure(error: unknown): boolean {
  return error instanceof Error
    && error.message.includes('encountered a connection error and is not queryable')
}

/**
 * Failures that came from the socket rather than from the server answering the
 * statement. The statement may or may not have executed — it is NEVER replayed —
 * but the connection is gone, so the next query must open a fresh one instead of
 * inheriting a corpse. Matched by message because pg surfaces these without a
 * stable error class; anything unrecognised stays untouched and propagates.
 */
function isConnectionLevelFailure(error: unknown): boolean {
  if (!(error instanceof Error)) {
    return false
  }
  const code = (error as { code?: unknown }).code
  // 57P01: "terminating connection due to administrator command" — the session was
  // killed, so the connection is dead even though the server answered first.
  if (code === '57P01' || code === 'ECONNRESET' || code === 'EPIPE') {
    return true
  }
  return error.message.includes('Connection terminated unexpectedly')
    || error.message.includes('socket hang up')
    || isUnsentConnectionFailure(error)
}

export interface PgOptions {
  /** The environment's PostgreSQL URL (`postgres://…`). Required — see the module doc. */
  readonly url: string
  readonly onLog?: (line: string) => void
}

/**
 * Connect to the environment's PostgreSQL. A blank/absent URL is a deployment
 * misconfiguration — this plugin never provisions a cluster of its own, so the
 * error names every place an operator can set one.
 */
export function acquirePostgres(options: PgOptions): Promise<PgHandle> {
  const url = options.url.trim()
  if (url.length === 0) {
    return Promise.reject(new Error(
      'env-alioth: no PostgreSQL URL configured. This plugin uses the environment\'s PostgreSQL 18 '
      + '(it no longer starts an embedded cluster). Set ALIOTH_DATABASE_URL — e.g. '
      + '`ALIOTH_DATABASE_URL=postgres://alioth@127.0.0.1:5432/alioth` in ~/.dsh-alioth.env (dev) or '
      + '/etc/dsh-alioth/env (prod) — or pass Config.databaseUrl',
    ))
  }
  return acquireExternal(url, options.onLog)
}



async function acquireExternal(url: string, onLog?: (line: string) => void): Promise<PgHandle> {
  const open = async (): Promise<Client> => {
    const client = new Client({ connectionString: url })
    await client.connect()
    return client
  }
  return createHandle({ initial: await open(), url, reconnect: open, onLog })
}

interface HandleParts {
  readonly initial: Client
  readonly url: string
  /** Open a replacement connection after the previous one died. */
  readonly reconnect: () => Promise<Client>
  readonly onLog?: ((line: string) => void) | undefined
}

function createHandle(parts: HandleParts): PgHandle {
  let current = parts.initial
  const arm = (client: Client): void => {
    // Without a listener a dying idle socket emits an unhandled 'error' and takes the
    // process down; with one, the failure is recorded and the next query reconnects.
    client.on('error', (error: unknown) => {
      parts.onLog?.(`env-alioth: db connection error (${String(error)}) — the next query reconnects`)
    })
  }
  arm(current)
  const replace = async (): Promise<Client> => {
    const replacement = await parts.reconnect()
    arm(replacement)
    current = replacement
    return replacement
  }
  const query: QueryFn = async (text, values) => {
    const args = values === undefined ? undefined : [...values]
    try {
      return await current.query(text, args)
    } catch (error) {
      if (isUnsentConnectionFailure(error)) {
        // Refused before sending: replaying is safe and keeps a long session alive.
        parts.onLog?.('env-alioth: db connection was dead — reconnecting once')
        return await (await replace()).query(text, args)
      }
      if (isConnectionLevelFailure(error)) {
        // May have executed: never replayed. Drop the corpse so the *next* query
        // (e.g. the next pipeline stage) reconnects instead of failing forever.
        parts.onLog?.('env-alioth: db connection lost — the next query reconnects')
        current = await replace()
      }
      throw error
    }
  }
  return {
    query,
    url: parts.url,
    close: async () => {
      await current.end().catch(() => {})
    },
  }
}

