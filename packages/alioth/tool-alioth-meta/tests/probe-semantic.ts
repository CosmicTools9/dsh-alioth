import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { Client } from 'pg'
import { ensureSemanticIndex, TransformersEmbedder, type SemanticEntry } from '../src/embedding.ts'

const client = new Client({ connectionString: process.env.DATABASE_URL ?? 'postgres://isahl@localhost/aliothstudio_dev' })
await client.connect()
const entities = await client.query<{ table_name: string; name: string; biz_description: string | null }>(
  `SELECT table_name, name, biz_description FROM isahl_meta.meta_collections
   WHERE table_name NOT LIKE '%-testing' AND table_name NOT LIKE '%-test'`,
)
const fields = await client.query<{ fk_collection: string; name: string; title: string }>(
  `SELECT fk_collection, name, title FROM isahl_meta.meta_fields WHERE title != ''`,
)
await client.end()

const entries: SemanticEntry[] = [
  ...entities.rows.map(row => ({ kind: 'entity' as const, table: row.table_name, name: row.name, title: row.biz_description ?? '' })),
  ...fields.rows.map(row => ({ kind: 'field' as const, table: row.fk_collection, name: row.name, title: row.title })),
]
console.log(`entries: ${entries.length}`)

const dataRoot = process.env.DATA_ROOT ?? await mkdtemp(path.join(tmpdir(), 'semantic-real-'))
console.log(`dataRoot: ${dataRoot}`)
const index = await ensureSemanticIndex(dataRoot, entries, new TransformersEmbedder())

const probes = ['库存余额', 'inventory balance', '采购申请', '负责人', '员工信息']
for (const probe of probes) {
  const hits = await index.search(probe, 5)
  console.log(`\n== ${probe} ==`)
  for (const hit of hits) {
    console.log(`  ${hit.score.toFixed(3)}  ${hit.entry.kind}  ${hit.entry.table}  ${hit.entry.name}${hit.entry.title ? ` / ${hit.entry.title}` : ''}`)
  }
}
