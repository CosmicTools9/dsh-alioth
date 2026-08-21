/**
 * PostgreSQL lifecycle for the Alioth environment. Two paths:
 * - `url` given → reuse an existing server (e.g. a developer's AliothStudio DB).
 * - no `url` → auto-provision an embedded PostgreSQL under `<dataRoot>/postgres`
 *   (real PG binaries via `embedded-postgres`): first run `initdb`s and creates
 *   the `alioth` database; later runs skip `initdb` and restart the persisted
 *   cluster on a freshly probed port.
 * @module @dsh-alioth/env-alioth/pg
 */

import { access } from 'node:fs/promises'
import net from 'node:net'
import path from 'node:path'
import EmbeddedPostgres from 'embedded-postgres'
import { Client } from 'pg'

/** A connected single client plus the URL it came from. */
export interface PgHandle {
  readonly client: Client
  /** Connection URL (contains credentials — mask before display). */
  readonly url: string
  /** Close the client and, when we own it, stop the embedded server. */
  close(): Promise<void>
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

async function acquireExternal(url: string): Promise<PgHandle> {
  const client = new Client({ connectionString: url })
  await client.connect()
  return { client, url, close: () => client.end() }
}

async function acquireEmbedded(options: PgOptions): Promise<PgHandle> {
  const dataDir = path.join(options.dataRoot, 'postgres')
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
  return {
    client,
    url,
    close: async () => {
      await client.end()
      await stopInstance()
    },
  }
}

/** Connect per `options`: external URL when given, else a provisioned embedded cluster. */
export function acquirePostgres(options: PgOptions): Promise<PgHandle> {
  return options.url === undefined ? acquireEmbedded(options) : acquireExternal(options.url)
}
