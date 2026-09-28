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
 * One task at a time, FIFO: a queued task starts only after the previous one
 * settled, and a rejection settles the lane without breaking it.
 *
 * The handle funnels every statement through one lane because the connection it
 * owns is a single `pg.Client`: pg rejects a second `client.query()` issued while
 * one is in flight ("Calling client.query() when the client is already executing
 * a query" — a warning in pg 8, a hard error from pg 9), and the replay predicates
 * below only hold while nothing else is in flight: a statement pg refused to send
 * may be replayed, one that merely failed must never be.
 * @returns enqueue function: `(task) => task()` serialized onto the lane.
 */
export function createSerialLane(): <T>(task: () => Promise<T>) => Promise<T> {
  let tail: Promise<unknown> = Promise.resolve()
  return <T>(task: () => Promise<T>): Promise<T> => {
    const run = tail.then(task, task)
    tail = run.then(
      () => undefined,
      () => undefined,
    )
    return run
  }
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
      + '`ALIOTH_DATABASE_URL=postgres://alioth@127.0.0.1:5432/dsh_alioth` in ~/.dsh-alioth.env (dev) or '
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
  const lane = createSerialLane()
  const dispatch = async <T extends QueryResultRow>(
    text: string,
    args: readonly unknown[] | undefined,
  ): Promise<QueryResult<T>> => {
    try {
      return await current.query<T>(text, args === undefined ? undefined : [...args])
    } catch (error) {
      if (isUnsentConnectionFailure(error)) {
        // Refused before sending: replaying is safe and keeps a long session alive.
        parts.onLog?.('env-alioth: db connection was dead — reconnecting once')
        return await (await replace()).query<T>(text, args === undefined ? undefined : [...args])
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
  const query: QueryFn = (text, values) => {
    const args = values === undefined ? undefined : [...values]
    return lane(() => dispatch(text, args))
  }
  return {
    query,
    url: parts.url,
    close: async () => {
      // Drain the lane first: ending the client under a queued statement would
      // fail that statement for a reason the caller cannot act on.
      await lane(async () => {}).catch(() => {})
      await current.end().catch(() => {})
    },
  }
}

