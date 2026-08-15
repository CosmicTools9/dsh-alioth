/**
 * `alioth_entity_write` tool: register a new business entity on an existing
 * isahl physical table (isahl forbids CREATE TABLE). Deterministic main path:
 * full validation (physical table, naming, conflicts, inheritance,
 * reference integrity, real coordinate dictionary) → optional approval →
 * INSERT meta_collections + meta_fields. No LLM in this path.
 * @module @dsh-alioth/tool-alioth-meta/entity-write
 */

import type { Context } from '@deepseek-ai/cordis'
import { defineTool } from '@deepseek-ai/dsh-tools'
import { validateEntitySpec, type EntitySpec, type FieldSpec, type RegistryView } from '@dsh-alioth/skill-alioth'
import type {} from '@deepseek-ai/dsh-user-approval'

const FIELD_CATEGORIES = ['scalar', 'reference', 'computed', 'auto'] as const

/** Registry view for the validators: table → name + inherits from config jsonb. */
async function loadRegistryView(ctx: Context): Promise<RegistryView> {
  const rows = await ctx.aliothEnv.sql<{ table_name: string; name: string; inherits: unknown }>(
    `SELECT table_name, name, config->'inherits' AS inherits
     FROM isahl_meta.meta_collections`,
  )
  const collections = new Map<string, { name: string; inherits: readonly string[] }>()
  for (const row of rows.rows) {
    collections.set(row.table_name, {
      name: row.name,
      inherits: Array.isArray(row.inherits) ? row.inherits.map(entry => String(entry)) : [],
    })
  }
  return { collections }
}

/** 1 + max parent depth from the registry; unknown parents count as depth 1. */
async function computeInheritanceDepth(ctx: Context, spec: EntitySpec): Promise<number> {
  const parents = spec.inherits
  if (parents.length === 0) {
    return 1
  }
  const rows = await ctx.aliothEnv.sql<{ table_name: string; depth: string | null }>(
    `SELECT table_name, config->>'depth' AS depth
     FROM isahl_meta.meta_collections
     WHERE table_name = ANY($1::text[])`,
    [parents],
  )
  const depthByTable = new Map(rows.rows.map(row => [row.table_name, Number(row.depth ?? 1)]))
  let maxParentDepth = 0
  for (const parent of parents) {
    const depth = depthByTable.get(parent) ?? 1
    if (depth > maxParentDepth) {
      maxParentDepth = depth
    }
  }
  return maxParentDepth + 1
}

function parseEntityFields(raw: unknown): readonly FieldSpec[] {
  if (raw === undefined) {
    return []
  }
  if (!Array.isArray(raw)) {
    throw new Error('alioth_entity_write: fields must be an array')
  }
  return raw.map((entry, index) => {
    if (typeof entry !== 'object' || entry === null) {
      throw new Error(`alioth_entity_write: field #${index} must be an object`)
    }
    const record = entry as Record<string, unknown>
    const name = record.name
    const category = record.category
    const dataType = record.dataType
    if (typeof name !== 'string' || name.length === 0) {
      throw new Error(`alioth_entity_write: field #${index} requires a name`)
    }
    if (typeof category !== 'string' || !(FIELD_CATEGORIES as readonly string[]).includes(category)) {
      throw new Error(`alioth_entity_write: field ${name} category must be one of ${FIELD_CATEGORIES.join(', ')}`)
    }
    if (typeof dataType !== 'string' || dataType.length === 0) {
      throw new Error(`alioth_entity_write: field ${name} requires dataType`)
    }
    const targetTable = record.targetTable
    const localKey = record.localKey
    const junctionTable = record.junctionTable
    const hasReference = targetTable !== undefined || localKey !== undefined || junctionTable !== undefined
    if (hasReference && category !== 'reference') {
      throw new Error(`alioth_entity_write: field ${name} declares reference targets but category is ${category}`)
    }
    if (category === 'reference' && targetTable === undefined) {
      throw new Error(`alioth_entity_write: reference field ${name} requires targetTable`)
    }
    return {
      name,
      category: category as FieldSpec['category'],
      dataType,
      ...(typeof record.required === 'boolean' ? { required: record.required } : {}),
      ...(typeof record.title === 'string' ? { title: record.title } : {}),
      ...(hasReference
        ? {
          reference: {
            targetTable: String(targetTable),
            ...(localKey !== undefined ? { localKey: String(localKey) } : {}),
            ...(junctionTable !== undefined ? { junctionTable: String(junctionTable) } : {}),
          },
        }
        : {}),
    }
  })
}

/** Register the `alioth_entity_write` tool. */
export function registerEntityWrite(ctx: Context, approvalMode: 'required' | 'bypass'): void {
  ctx.tools.register(defineTool({
    name: 'alioth_entity_write',
    description:
      'Register a new business entity on an existing isahl physical table (isahl forbids CREATE TABLE): '
      + 'INSERTs meta_collections + meta_fields rows. Every definition passes the write-path validators '
      + 'first — physical table existence, naming, conflicts, inheritance (exists/acyclic/depth), '
      + 'reference integrity (local_key vs the FK index), and real coordinate-dictionary codes; any '
      + 'violation fails the whole write with all issues listed. Requires approval when the deployment '
      + 'sets approvalMode=required. Query the registry with alioth_schema_info / alioth_schema_semantic_search '
      + 'before defining; pick an unregistered physical table (list: schema_info entities) for the new entity. '
      + 'NOTE: registration is a consumption-side extension of the bootstrapped registry — this plugin '
      + 'cannot advance the Alioth model (no new physical tables, no version changes); the registry is '
      + 'rebuilt from the published baseline on model refresh.',
    parameters: {
      table: {
        type: 'string',
        required: true,
        description: 'isahl physical table name the entity maps onto (e.g. zc_id_purchase-order).',
      },
      name: {
        type: 'string',
        required: true,
        description: 'Business entity name (e.g. 采购订单).',
      },
      inherits: {
        type: 'array',
        items: { type: 'string' },
        description: 'Parent tables/entities (e.g. zc_id_object). Depth is validated against the snapshot limit.',
      },
      category: {
        type: 'string',
        description: 'Business category name stored in config.category (e.g. 交易信息).',
      },
      coordinates: {
        type: 'object',
        additionalProperties: false,
        properties: {
          scene: { type: 'string', required: true },
          factor: { type: 'string', required: true },
          function: { type: 'string', required: true },
        },
        description: 'Ontology coordinates; codes are validated against the real dictionary.',
      },
      fields: {
        type: 'array',
        items: {
          type: 'object',
          additionalProperties: false,
          properties: {
            name: { type: 'string', required: true },
            category: { type: 'string', required: true },
            dataType: { type: 'string', required: true },
            title: { type: 'string' },
            required: { type: 'boolean' },
            targetTable: { type: 'string' },
            localKey: { type: 'string' },
            junctionTable: { type: 'string' },
          },
        },
        description: 'Fields to register. Reference fields (category=reference) declare targetTable and '
          + 'optionally localKey (physical FK column, checked against the FK index) or junctionTable.',
      },
    },
    output: {
      schema: {
        type: 'object',
        additionalProperties: false,
        properties: {
          table: { type: 'string', required: true },
          name: { type: 'string', required: true },
          fields: { type: 'number', required: true },
        },
      },
      render: (_args, value) => [{
        type: 'text',
        text: `Registered entity ${value.table} (${value.name}) with ${value.fields} fields`,
      }],
    },
    async execute(args, exec) {
      const fields = parseEntityFields(args.fields)
      const spec: EntitySpec = {
        table: args.table,
        name: args.name,
        inherits: args.inherits ?? [],
        ...(args.coordinates === undefined ? {} : { coordinates: args.coordinates }),
        fields,
      }
      const registry = await loadRegistryView(ctx)
      const issues = validateEntitySpec(spec, registry)
      if (issues.length > 0) {
        throw new Error(`alioth_entity_write: definition invalid:\n${issues.map(issue => `- [${issue.code}] ${issue.message}`).join('\n')}`)
      }

      if (approvalMode === 'required') {
        const approval = ctx.get('approval')
        if (approval === undefined) {
          throw new Error('alioth_entity_write: approvalMode=required but no ApprovalService is composed')
        }
        if (exec.agent === undefined) {
          throw new Error('alioth_entity_write: approvalMode=required but the call has no agent to route approval')
        }
        const outcome = await approval.request({
          agent: exec.agent,
          toolName: 'alioth_entity_write',
          callId: exec.callId,
          reason: `Register entity ${args.table} (${args.name}) in isahl_meta`,
          signal: exec.signal,
        })
        if (outcome !== 'allowed-once') {
          throw new Error(`alioth_entity_write: denied by approval (${outcome})`)
        }
      }

      const depth = await computeInheritanceDepth(ctx, spec)
      const config: Record<string, unknown> = {
        depth,
        source: 'dsh-alioth',
        inherits: [...spec.inherits],
        ...(args.category === undefined ? {} : { category: { name: args.category } }),
        ...(spec.coordinates === undefined ? {} : { coordinates: spec.coordinates }),
      }
      await ctx.aliothEnv.sql(
        `INSERT INTO isahl_meta.meta_collections (table_name, name, type, config, data_source, schema)
         VALUES ($1, $2, 'table', $3::jsonb, 'dsh-alioth', 'isahl')`,
        [spec.table, spec.name, JSON.stringify(config)],
      )
      for (const field of spec.fields) {
        const fieldConfig = field.reference === undefined ? {} : { reference_config: field.reference }
        await ctx.aliothEnv.sql(
          `INSERT INTO isahl_meta.meta_fields
             (fk_collection, name, category, data_type, is_required, default_value, config, title)
           VALUES ($1, $2, $3, $4, $5, NULL, $6::jsonb, $7)`,
          [
            spec.table,
            field.name,
            field.category,
            field.dataType,
            field.required ?? false,
            JSON.stringify(fieldConfig),
            field.title ?? '',
          ],
        )
      }
      return { table: spec.table, name: spec.name, fields: spec.fields.length }
    },
    presentCall: args => ({
      card: 'generic',
      title: `Register Alioth entity ${String(args.table)}`,
      kind: 'other',
      rawInput: args as Record<string, unknown>,
    }),
  }))
}
