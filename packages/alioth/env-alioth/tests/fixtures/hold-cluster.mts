/**
 * Test-support fixture: acquires the embedded cluster for `dataRoot` and holds
 * it alive — simulating another dsh instance owning the data root. Prints
 * `HOLD-CLUSTER-READY` on stdout once acquired; auto-releases after `ttlMs`.
 * Usage: node --import tsx hold-cluster.mts <dataRoot> [ttlMs]
 */
import { acquirePostgres } from '../../src/pg.ts'

const dataRoot = process.argv[2]
const ttlMs = Number(process.argv[3] ?? 12_000)

if (dataRoot === undefined || dataRoot === '') {
  console.error('usage: hold-cluster.mts <dataRoot> [ttlMs]')
  process.exit(1)
}

const handle = await acquirePostgres({ dataRoot })
const probe = await handle.client.query('select 1 as ok')
console.log(`HOLD-CLUSTER-READY probe=${probe.rows[0]?.ok}`)
setTimeout(() => {
  void handle.close().then(() => process.exit(0))
}, ttlMs)
