/**
 * PostgreSQL lifecycle for the Alioth environment. Two paths:
 * - `url` given → reuse an existing server (e.g. a developer's AliothStudio DB).
 * - no `url` → auto-provision an embedded PostgreSQL under `<dataRoot>/postgres`
 *   (real PG binaries via `embedded-postgres`): first run `initdb`s and creates
 *   the `alioth` database; later runs skip `initdb` and restart the persisted
 *   cluster on a freshly probed port.
 * @module @dsh-alioth/env-alioth/pg
 */

import { access, readFile } from 'node:fs/promises'
import net from 'node:net'
import path from 'node:path'
import EmbeddedPostgres from 'embedded-postgres'
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
  /** Close the connection and, when we own it, stop the embedded server. */
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
  /** Reuse an existing PostgreSQL; omit to auto-provision under `dataRoot`. */
  readonly url?: string
  /** State root; the embedded cluster lives at `<dataRoot>/postgres`. */
  readonly dataRoot: string
  /** Receives embedded-server process output (initdb/postgres logs). */
  readonly onLog?: (line: string) => void
}

const EMBEDDED_USER = 'alioth'
const EMBEDDED_PASSWORD = 'alioth'
const EMBEDDED_DATABASE = 'alioth'

/** Probe an OS-assigned free TCP port (listen on :0, read it, release). */
async function reservePort(): Promise<number> {
  const { promise, resolve, reject } = Promise.withResolvers<number>()
  const server = net.createServer()
  server.unref()
  server.once('error', reject)
  server.listen(0, '127.0.0.1', () => {
    const address = server.address()
    if (address === null || typeof address === 'string') {
      server.close(() => reject(new Error('env-alioth: no port assigned')))
      return
    }
    const { port } = address
    server.close(() => resolve(port))
  })
  return promise
}

async function pathExists(target: string): Promise<boolean> {
  try {
    await access(target)
    return true
  } catch {
    return false
  }
}

/**
 * Fail loud when the cluster's data dir is held by a LIVE postmaster.
 * Without this guard the failure mode is a silent infinite hang: the start
 * attempt fails, and the `stop()` in the retry path waits for the OTHER
 * instance's healthy postmaster to exit — which never happens.
 * A stale lock (dead pid) is left for postgres itself to clear on start.
 */
async function assertClusterFree(dataDir: string): Promise<void> {
  const lockFile = path.join(dataDir, 'postmaster.pid')
  if (!await pathExists(lockFile)) {
    return
  }
  const firstLine = (await readFile(lockFile, 'utf8')).split('\n')[0] ?? ''
  const pid = Number.parseInt(firstLine, 10)
  if (!Number.isInteger(pid) || pid <= 0) {
    return
  }
  let alive = true
  try {
    process.kill(pid, 0)
  } catch (error) {
    alive = (error as NodeJS.ErrnoException).code === 'EPERM'
  }
  if (alive) {
    throw new Error(
      `env-alioth: the embedded cluster at ${dataDir} is already running (postmaster pid ${pid}). `
      + 'Another dsh instance holds this data root — stop that instance first, '
      + 'or point this deployment at a different data root (ALIOTH_DATA_ROOT / Config.dataRoot).',
    )
  }
}

async function acquireExternal(url: string, onLog?: (line: string) => void): Promise<PgHandle> {
  const open = async (): Promise<Client> => {
    const client = new Client({ connectionString: url })
    await client.connect()
    return client
  }
  return createHandle({ initial: await open(), url, reconnect: open, onLog, release: async () => {} })
}

async function acquireEmbedded(options: PgOptions): Promise<PgHandle> {
  const dataDir = path.join(options.dataRoot, 'postgres')
  await assertClusterFree(dataDir)
  const fresh = !await pathExists(path.join(dataDir, 'PG_VERSION'))
  // reservePort is TOCTOU (probe port, release, then PG binds): under
  // parallel boot (test suite) the probed port can be taken between probe and
  // bind, and a just-stopped sibling cluster may not have released its port
  // yet — retry with a fresh port instead of failing the whole boot.
  let initialised = false
  let instance: EmbeddedPostgres | undefined
  let usedPort = 0
  let lastError: unknown
  for (let attempt = 1; attempt <= 3 && instance === undefined; attempt++) {
    const port = await reservePort()
    const candidate = new EmbeddedPostgres({
      databaseDir: dataDir,
      port,
      user: EMBEDDED_USER,
      password: EMBEDDED_PASSWORD,
      authMethod: 'password',
      persistent: true,
      onLog: line => options.onLog?.(line),
      onError: message => options.onLog?.(String(message)),
    })
    try {
      if (fresh && !initialised) {
        await candidate.initialise()
        initialised = true
      }
      await candidate.start()
      instance = candidate
      usedPort = port
    } catch (error) {
      lastError = error
      await candidate.stop().catch(() => {})
      options.onLog?.(`env-alioth: embedded PG start attempt ${attempt} failed (${String(error)}) — retrying on a fresh port`)
    }
  }
  if (instance === undefined) {
    throw lastError ?? new Error('env-alioth: embedded PG failed to start after 3 attempts')
  }
  const pg = instance
  const url = `postgres://${EMBEDDED_USER}:${EMBEDDED_PASSWORD}@127.0.0.1:${usedPort}/${EMBEDDED_DATABASE}`

  async function connectWithCreate(): Promise<Client> {
    const client = pg.getPgClient(EMBEDDED_DATABASE)
    try {
      await client.connect()
      return client
    } catch (error) {
      // "database ... does not exist": a persisted cluster that never got the
      // `alioth` database (foreign data dir, or interrupted first run).
      if (!(error instanceof Error) || !error.message.includes('does not exist')) {
        throw error
      }
      await pg.createDatabase(EMBEDDED_DATABASE)
      const retry = pg.getPgClient(EMBEDDED_DATABASE)
      await retry.connect()
      return retry
    }
  }

  async function stopInstance(): Promise<void> {
    await pg.stop()
  }

  const client = await connectWithCreate().catch(async (error: unknown) => {
    await stopInstance()
    throw error
  })
  return createHandle({
    initial: client,
    url,
    reconnect: connectWithCreate,
    onLog: options.onLog,
    release: stopInstance,
  })
}

interface HandleParts {
  readonly initial: Client
  readonly url: string
  /** Open a replacement connection after the previous one died. */
  readonly reconnect: () => Promise<Client>
  readonly onLog?: ((line: string) => void) | undefined
  /** Release what we own (the embedded server, when we own one). */
  readonly release: () => Promise<void>
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
      await parts.release()
    },
  }
}

/** Connect per `options`: external URL when given, else a provisioned embedded cluster. */
export function acquirePostgres(options: PgOptions): Promise<PgHandle> {
  return options.url === undefined ? acquireEmbedded(options) : acquireExternal(options.url, options.onLog)
}
