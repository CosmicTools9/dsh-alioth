/**
 * `alioth_schema_info` tool: literal registry queries (entities / entity /
 * search-fields) over `ctx.aliothEnv.sql()`. Dev-seed test entities
 * (`-testing`/`-test` suffixes) are hidden by default with a transparency
 * count; `includeTesting` opts back in.
 * @module @dsh-alioth/tool-alioth-meta/schema-info
 */

import type { Context } from '@deepseek-ai/cordis'
import { defineTool } from '@deepseek-ai/dsh-tools'

interface CollectionRow {
  readonly table_name: string
  readonly name: string
  readonly type: string | null
  readonly depth: string | null
  readonly category: string | null
  readonly inherits: unknown
  readonly biz_description: string | null
}

interface FieldRow {
  readonly fk_collection: string
  readonly name: string
  readonly title: string
  readonly category: string | null
  readonly data_type: string | null
  readonly is_required: boolean | null
  readonly default_value: string | null
}

const ACTIONS = ['entities', 'entity', 'search-fields'] as const

const MAX_LIMIT = 100
const DEFAULT_LIMIT = 20

import { testEntityFilter } from './hygiene.ts'

const TEST_ENTITY_FILTER = testEntityFilter()

const ENTITIES_SQL = `
SELECT table_name, name, type,
       config->>'depth' AS depth,
       config->'category'->>'name' AS category,
       config->'inherits' AS inherits,
       biz_description
FROM isahl_meta.meta_collections
WHERE ($1::text IS NULL OR table_name ILIKE '%' || $1 || '%' OR name ILIKE '%' || $1 || '%')
  AND ($2::text IS NULL OR config->'category'->>'name' = $2)
  AND ($3::boolean OR ${TEST_ENTITY_FILTER})
ORDER BY table_name
LIMIT $4`

/** How many test entities the current filter hides, for transparency. */
const TESTING_COUNT_SQL = `
SELECT count(*)::int AS hidden
FROM isahl_meta.meta_collections
WHERE table_name LIKE '%-testing' OR table_name LIKE '%-test'`

const COLLECTION_SQL = `
SELECT table_name, name, type,
       config->>'depth' AS depth,
       config->'category'->>'name' AS category,
       config->'inherits' AS inherits,
       biz_description
FROM isahl_meta.meta_collections
WHERE table_name = $1`

const FIELDS_SQL = `
SELECT fk_collection, name, title, category, data_type, is_required, default_value
FROM isahl_meta.meta_fields
WHERE fk_collection = $1
ORDER BY name`

const SEARCH_SQL = `
SELECT mf.fk_collection, mf.name, mf.title, mf.category, mf.data_type
FROM isahl_meta.meta_fields mf
JOIN isahl_meta.meta_collections mc ON mc.table_name = mf.fk_collection
WHERE (mf.name ILIKE '%' || $1 || '%' OR mf.title ILIKE '%' || $1 || '%')
  AND ($2::boolean OR (mc.table_name NOT LIKE '%-testing' AND mc.table_name NOT LIKE '%-test'))
ORDER BY mf.fk_collection, mf.name
LIMIT $3`

/** jsonb `config->'inherits'` arrives as a parsed JSON array or null. */
function toInherits(value: unknown): string[] {
  return Array.isArray(value) ? value.map(entry => String(entry)) : []
}

/** Register the `alioth_schema_info` tool. */
export function registerSchemaInfo(ctx: Context): void {
  ctx.tools.register(defineTool({
    name: 'alioth_schema_info',
    description:
      'Query the Alioth entity registry (isahl_meta). Actions: '
      + '"entities" — list collections (optional substring `query` on table/name, exact `category`); '
      + '"entity" — one collection\'s fields (`collection` = table_name); '
      + '"search-fields" — fields by substring on name/title (`query`). '
      + 'Use before defining or referencing any entity/field; the registry is the structural truth. '
      + 'Dev-seed test entities (names ending -testing/-test) are hidden by default '
      + '(`filteredTesting` reports how many); pass `includeTesting` to see them. '
      + 'Returns at most `limit` rows (default 20, max 100).',
    parameters: {
      action: {
        type: 'string',
        required: true,
        description: `One of: ${ACTIONS.join(', ')}.`,
      },
      query: {
        type: 'string',
        description: 'Substring filter (entities: table_name or name; search-fields: field name or title).',
      },
      category: {
        type: 'string',
        description: 'entities only: exact category name from config.category.name (e.g. "交易信息").',
      },
      collection: {
        type: 'string',
        description: 'entity only: collection table_name, e.g. "zc_id_inventory".',
      },
      limit: {
        type: 'number',
        description: `Max rows (default ${DEFAULT_LIMIT}, max ${MAX_LIMIT}).`,
      },
      includeTesting: {
        type: 'boolean',
        description: 'entities/search-fields only: include dev-seed test entities (table names ending -testing/-test); default false.',
      },
    },
    output: {
      schema: {
        type: 'object',
        additionalProperties: false,
        properties: {
          action: { type: 'string', required: true },
          filteredTesting: { type: 'number' },
          entities: {
            type: 'array',
            items: {
              type: 'object',
              additionalProperties: false,
              properties: {
                table: { type: 'string', required: true },
                name: { type: 'string', required: true },
                type: { type: 'string', required: true },
                category: { type: 'string', required: true },
                depth: { type: 'number', required: true },
                inherits: { type: 'array', required: true, items: { type: 'string' } },
                description: { type: 'string', required: true },
              },
            },
          },
          collection: {
            type: 'object',
            additionalProperties: false,
            properties: {
              table: { type: 'string', required: true },
              name: { type: 'string', required: true },
              category: { type: 'string', required: true },
              inherits: { type: 'array', required: true, items: { type: 'string' } },
              description: { type: 'string', required: true },
            },
          },
          fields: {
            type: 'array',
            items: {
              type: 'object',
              additionalProperties: false,
              properties: {
                collection: { type: 'string', required: true },
                name: { type: 'string', required: true },
                title: { type: 'string', required: true },
                category: { type: 'string', required: true },
                dataType: { type: 'string', required: true },
                required: { type: 'boolean', required: true },
                defaultValue: { type: 'string', required: true },
              },
            },
          },
        },
      },
      render: (_args, value) => [{
        type: 'text',
        text: value.action === 'entities'
          ? `${value.entities?.length ?? 0} collections${value.entities?.length ? ': ' + value.entities.map(e => e.table).join(', ') : ''}`
          : value.action === 'entity'
            ? `${value.collection?.table}: ${value.fields?.length ?? 0} fields`
            : `${value.fields?.length ?? 0} fields matching: ${value.fields?.map(f => `${f.collection}.${f.name}`).join(', ')}`,
      }],
    },
    async execute(args) {
      const action = args.action as string
      if (!(ACTIONS as readonly string[]).includes(action)) {
        throw new Error(`alioth_schema_info: invalid action ${JSON.stringify(args.action)} (expected ${ACTIONS.join(', ')})`)
      }
      const limit = args.limit === undefined ? DEFAULT_LIMIT : args.limit
      if (!Number.isInteger(limit) || limit < 1 || limit > MAX_LIMIT) {
        throw new Error(`alioth_schema_info: limit must be an integer in [1, ${MAX_LIMIT}]`)
      }

      if (action === 'entities') {
        const includeTesting = args.includeTesting === true
        const res = await ctx.aliothEnv.sql<CollectionRow>(
          ENTITIES_SQL,
          [args.query ?? null, args.category ?? null, includeTesting, limit],
        )
        const hidden = includeTesting ? 0 : (await ctx.aliothEnv.sql<{ hidden: number }>(TESTING_COUNT_SQL, [])).rows[0]?.hidden ?? 0
        return {
          action,
          filteredTesting: hidden,
          entities: res.rows.map(row => ({
            table: row.table_name,
            name: row.name,
            type: row.type ?? '',
            category: row.category ?? '',
            depth: Number(row.depth ?? 0),
            inherits: toInherits(row.inherits),
            description: row.biz_description ?? '',
          })),
        }
      }

      if (action === 'entity') {
        if (typeof args.collection !== 'string' || args.collection.length === 0) {
          throw new Error('alioth_schema_info: action "entity" requires "collection"')
        }
        const head = await ctx.aliothEnv.sql<CollectionRow>(COLLECTION_SQL, [args.collection])
        const row = head.rows[0]
        if (row === undefined) {
          throw new Error(`alioth_schema_info: unknown collection ${JSON.stringify(args.collection)} — list with action "entities" first`)
        }
        const fields = await ctx.aliothEnv.sql<FieldRow>(FIELDS_SQL, [args.collection])
        return {
          action,
          collection: {
            table: row.table_name,
            name: row.name,
            category: row.category ?? '',
            inherits: toInherits(row.inherits),
            description: row.biz_description ?? '',
          },
          fields: fields.rows.map(field => ({
            collection: field.fk_collection,
            name: field.name,
            title: field.title,
            category: field.category ?? '',
            dataType: field.data_type ?? '',
            required: field.is_required ?? false,
            defaultValue: field.default_value ?? '',
          })),
        }
      }

      if (typeof args.query !== 'string' || args.query.length === 0) {
        throw new Error('alioth_schema_info: action "search-fields" requires "query"')
      }
      const includeTesting = args.includeTesting === true
      const found = await ctx.aliothEnv.sql<FieldRow>(SEARCH_SQL, [args.query, includeTesting, limit])
      return {
        action,
        fields: found.rows.map(field => ({
          collection: field.fk_collection,
          name: field.name,
          title: field.title,
          category: field.category ?? '',
          dataType: field.data_type ?? '',
          required: field.is_required ?? false,
          defaultValue: field.default_value ?? '',
        })),
      }
    },
    presentCall: args => ({
      card: 'generic',
      title: `Alioth registry ${String(args.action)}`,
      kind: 'other',
      rawInput: args as Record<string, unknown>,
    }),
  }))
}
