/**
 * 运营单命令：授予/撤销管理员角色（`dsh_alioth_auth.users.role`）。
 *
 * 与 `source:grant` 同一形状：直连 `ALIOTH_DATABASE_URL`，不拉模型快照、
 * 不经插件面。在线 B/S 面上没有任何 API 能造出 admin（人人平等、无超管
 * 自助升级），管理员由部署运营用这条命令显式指定——这是唯一通道。
 * 连接走 env-alioth 的查询漏斗（`acquirePostgres`），不自持裸 Client。
 *
 *   pnpm run admin:grant --user <用户名>            # 设为 admin
 *   pnpm run admin:grant --user <用户名> --revoke   # 撤销为 user
 *   pnpm run admin:grant --list                     # 现有管理员清单
 * @module scripts/grant-admin-role
 */

import { acquirePostgres } from '@dsh-alioth/env-alioth'

const AUTH_SCHEMA = 'dsh_alioth_auth'

function fail(message: string): never {
  console.error(`admin:grant: ${message}`)
  process.exit(1)
}

function argOf(name: string): string | undefined {
  const index = process.argv.indexOf(name)
  return index === -1 ? undefined : process.argv[index + 1]
}

async function main(): Promise<void> {
  const url = process.env.ALIOTH_DATABASE_URL
  if (url === undefined || url === '') {
    fail('ALIOTH_DATABASE_URL is required (set it in ~/.dsh-alioth.env or the environment)')
  }
  const handle = await acquirePostgres({ url })
  try {
    await handle.query(`CREATE SCHEMA IF NOT EXISTS ${AUTH_SCHEMA}`)
    if (process.argv.includes('--list')) {
      const admins = await handle.query<{ username: string; created_at: string }>(
        `SELECT username, created_at FROM ${AUTH_SCHEMA}.users WHERE role = 'admin' ORDER BY username`,
      )
      if (admins.rows.length === 0) {
        console.log('no admins (every account is a plain user)')
        return
      }
      for (const row of admins.rows) {
        console.log(`${row.username}\t${row.created_at}`)
      }
      return
    }
    const username = argOf('--user')
    if (username === undefined || username === '') {
      fail('需要 --user <用户名>（或 --list）')
    }
    const revoke = process.argv.includes('--revoke')
    const role = revoke ? 'user' : 'admin'
    const existing = await handle.query<{ id: string; role: string }>(
      `SELECT id, role FROM ${AUTH_SCHEMA}.users WHERE username = $1`,
      [username],
    )
    if (existing.rows.length === 0) {
      fail(`用户不存在：${username}`)
    }
    if (existing.rows[0]!.role === role) {
      console.log(`${username} 已是 ${role}（幂等，未改动）`)
      return
    }
    await handle.query(`UPDATE ${AUTH_SCHEMA}.users SET role = $2 WHERE id = $1`, [existing.rows[0]!.id, role])
    console.log(`${username}: ${existing.rows[0]!.role} → ${role}`)
    console.log('审计提示：本次改动建议记录在案（管理员是跨命名空间的信任边界）。')
  } finally {
    await handle.close()
  }
}

await main()
