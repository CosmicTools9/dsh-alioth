/**
 * Operator command for L2 source-download authorizations.
 *
 * Source is 商务对接: someone agrees a window, and this is how the deployment
 * records it. The user center writes the request; this stamps the grant. It opens
 * a plain database connection — no model snapshot, no registry work.
 *
 *   pnpm run source:grant --user ada --until 2027-12-31 --note "合同 2026-114"
 *   pnpm run source:grant --list
 *
 * DSN comes from ALIOTH_DATABASE_URL (or --dsn), the same variable every
 * deployment already sets.
 */
import process from 'node:process'
import { acquirePostgres } from '@dsh-alioth/env-alioth'
import { createPgLicenseStore, ensureLicenseSchema } from '@dsh-alioth/billing-alioth'

interface Args {
  readonly user: string
  readonly until: string
  readonly note: string
  readonly list: boolean
  readonly dsn: string
}

function parse(argv: readonly string[]): Args {
  const read = (flag: string): string => {
    const index = argv.indexOf(flag)
    const value = index === -1 ? undefined : argv[index + 1]
    return value === undefined || value.startsWith('--') ? '' : value
  }
  return {
    user: read('--user'),
    until: read('--until'),
    note: read('--note'),
    list: argv.includes('--list'),
    dsn: read('--dsn') || process.env.ALIOTH_DATABASE_URL || '',
  }
}

/** Date-only means the whole day, matching the runtime's own interpretation. */
function parseUntil(value: string): Date {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(value)) {
    throw new Error(`--until 需要 YYYY-MM-DD，收到 ${JSON.stringify(value)}`)
  }
  return new Date(`${value}T23:59:59.999Z`)
}

async function main(): Promise<void> {
  const args = parse(process.argv.slice(2))
  if (args.dsn === '') {
    throw new Error('缺少数据库连接：设置 ALIOTH_DATABASE_URL 或传 --dsn')
  }
  const handle = await acquirePostgres({ url: args.dsn })
  // The operator command must work before the app ever served a request, so it
  // creates the store's schema itself (idempotent).
  await ensureLicenseSchema(handle.query)
  const licenses = createPgLicenseStore(handle.query)
  try {
    if (args.list) {
      const pending = await licenses.pending()
      if (pending.length === 0) {
        console.log('没有待开通的 L2 申请。')
        return
      }
      console.log(`待开通 L2 申请 ${pending.length} 条：`)
      for (const row of pending) {
        const username = await usernameOf(handle.query, row.userId)
        console.log(`  ${username ?? row.userId}（${row.userId}）  申请于 ${row.requestedAt.toISOString()}`)
      }
      console.log('\n开通：pnpm run source:grant --user <用户名> --until YYYY-MM-DD')
      return
    }

    if (args.user === '' || args.until === '') {
      throw new Error('用法：pnpm run source:grant --user <用户名> --until YYYY-MM-DD [--note 备注]  |  --list')
    }
    const users = await handle.query<{ id: string; username: string }>(
      'SELECT id, username FROM dsh_alioth_auth.users WHERE username = $1',
      [args.user],
    )
    const account = users.rows[0]
    if (account === undefined) {
      throw new Error(`账号不存在：${args.user}`)
    }
    const row = await licenses.grant(account.id, parseUntil(args.until), args.note)
    console.log(`已开通 L2 源码下载授权：${account.username}（${account.id}）至 ${row.until?.toISOString() ?? args.until}`)
    if (row.note !== '') console.log(`备注：${row.note}`)
  } finally {
    await handle.close()
  }
}

async function usernameOf(
  query: (text: string, values?: readonly unknown[]) => Promise<{ rows: Array<{ username?: string }> }>,
  userId: string,
): Promise<string | undefined> {
  const result = await query('SELECT username FROM dsh_alioth_auth.users WHERE id = $1', [userId])
  return result.rows[0]?.username
}

main().catch((error: unknown) => {
  // One line, exit 1: the message is the diagnosis an operator needs; a stack would bury it.
  console.error(`grant-source-license: ${error instanceof Error ? error.message : String(error)}`)
  process.exitCode = 1
})
