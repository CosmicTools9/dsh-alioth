/**
 * Model-facing inspection of an Alioth app artifact. Reads `app.json` from the configured
 * Pre-Proc tree, validates its required fields, and returns a structured summary — the
 * grounding an agent needs before creating or extending apps through the Alioth model
 * pipeline (AppCreator capability).
 * @module @dsh-alioth/tool-alioth
 */

import { readFile } from 'node:fs/promises'
import path from 'node:path'
import type { Context } from '@deepseek-ai/cordis'
import z from '@deepseek-ai/schemastery'
import { defineTool } from '@deepseek-ai/dsh-tools'

export const name = 'tool-alioth'
export const inject = ['tools']

/** Deployment choice: the Alioth Pre-Proc artifact tree root (e.g. `<repo>/Pre-Proc`). */
export interface Config {
  preProcRoot: string
}

/** Schemastery configuration for the alioth app-inspection tool consumer. */
export const Config: z<Config> = z.object({
  preProcRoot: z.string().required(),
})

/** Alioth namespace contract: `^[A-Z][a-zA-Z0-9-]*$` (Gateway runtime requirement). */
const NAMESPACE_PATTERN_RE = /^[A-Z][a-zA-Z0-9-]*$/

/** App code is a directory name under `Apps/`: letters, digits, hyphens only. */
const APP_PATTERN_RE = /^[a-zA-Z0-9][a-zA-Z0-9-]*$/

/** Fields an Alioth `app.json` must carry; anything else is reported, not rejected. */
const REQUIRED_FIELDS = ['id', 'code', 'namespace', 'name', 'version', 'config'] as const

function asString(value: unknown): string | null {
  return typeof value === 'string' ? value : null
}

function asStringArray(value: unknown): string[] {
  return Array.isArray(value) ? value.filter((item): item is string => typeof item === 'string') : []
}

/**
 * Register the `alioth_app_inspect` tool on `ctx.tools`.
 * @param ctx - registrant context carrying the tool registry.
 * @param config - deployment's explicit Pre-Proc root.
 */
export function apply(ctx: Context, config: Config): void {
  const root = path.resolve(config.preProcRoot)
  ctx.tools.register(defineTool({
    name: 'alioth_app_inspect',
    description:
      'Inspect an Alioth app artifact (app.json) under the configured Pre-Proc tree. '
      + 'Returns the app code, version, namespace, required modules and blocks, routing, '
      + 'navigation groups, roles, and any missing required fields. Use this before '
      + 'creating or extending an Alioth app so the model reads the real artifact — '
      + 'never guess an app\'s contents.',
    parameters: {
      namespace: {
        type: 'string',
        required: true,
        description: 'Alioth namespace, e.g. "Alioth". Letters, digits, hyphens only.',
      },
      app: {
        type: 'string',
        required: true,
        description: 'App code as in the directory name under Apps/, e.g. "ai-i-need-a". Letters, digits, hyphens only.',
      },
    },
    output: {
      schema: {
        type: 'object',
        additionalProperties: false,
        properties: {
          code: { type: 'string', required: true },
          name: { type: 'string', required: true },
          version: { type: 'string', required: true },
          namespace: { type: 'string', required: true },
          minAliothVersion: { type: 'string', required: true },
          modules: { type: 'array', required: true, items: { type: 'string' } },
          blocks: { type: 'array', required: true, items: { type: 'string' } },
          routing: {
            type: 'object', required: true, additionalProperties: false,
            properties: {
              base: { type: 'string', required: true },
              defaultRoute: { type: 'string', required: true },
            },
          },
          navigationGroups: { type: 'array', required: true, items: { type: 'string' } },
          roles: {
            type: 'object', required: true, additionalProperties: false,
            properties: {
              defaultRoles: { type: 'array', required: true, items: { type: 'string' } },
              adminRoles: { type: 'array', required: true, items: { type: 'string' } },
            },
          },
          missing: { type: 'array', required: true, items: { type: 'string' } },
        },
      },
      render: (_args, value) => [{
        type: 'text',
        text: `Alioth app ${value.namespace}/${value.code} v${value.version}: `
          + `${value.modules.length} modules, ${value.blocks.length} blocks`
          + (value.missing.length > 0 ? `, missing required: ${value.missing.join(', ')}` : ''),
      }],
    },
    async execute(args) {
      // The parameter schema DSL cannot express `pattern`, so validate the
      // free-form strings here — before any path resolution. This is the
      // security boundary that keeps reads inside preProcRoot.
      if (!NAMESPACE_PATTERN_RE.test(args.namespace)) {
        throw new Error(`alioth_app_inspect: invalid namespace ${JSON.stringify(args.namespace)} (expected ^[A-Z][a-zA-Z0-9-]*$)`)
      }
      if (!APP_PATTERN_RE.test(args.app)) {
        throw new Error(`alioth_app_inspect: invalid app code ${JSON.stringify(args.app)} (expected ^[a-zA-Z0-9][a-zA-Z0-9-]*$)`)
      }
      const resolved = path.resolve(root, args.namespace, 'Apps', args.app, 'app.json')
      // Invariant: the namespace/app patterns exclude separators, so the resolved path
      // always stays under preProcRoot. Assert it rather than silently widening.
      if (!resolved.startsWith(root + path.sep)) {
        throw new Error(`alioth_app_inspect: path escapes preProcRoot: ${resolved}`)
      }
      let raw: string
      try {
        raw = await readFile(resolved, 'utf8')
      } catch (error) {
        const isMissing = error instanceof Error
          && 'code' in error
          && (error as NodeJS.ErrnoException).code === 'ENOENT'
        throw new Error(isMissing
          ? `alioth_app_inspect: no app.json at ${resolved}`
          : `alioth_app_inspect: failed to read ${resolved}: ${error instanceof Error ? error.message : String(error)}`)
      }
      let parsed: unknown
      try {
        parsed = JSON.parse(raw)
      } catch (error) {
        throw new Error(`alioth_app_inspect: invalid JSON in ${resolved}: ${error instanceof Error ? error.message : String(error)}`)
      }
      if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) {
        throw new Error(`alioth_app_inspect: app.json must be a JSON object: ${resolved}`)
      }
      const record = parsed as Record<string, unknown>
      const configObj = typeof record.config === 'object' && record.config !== null
        ? record.config as Record<string, unknown>
        : {}
      const routingObj = typeof record.routing === 'object' && record.routing !== null
        ? record.routing as Record<string, unknown>
        : {}
      const permissions = typeof record.permissions === 'object' && record.permissions !== null
        ? record.permissions as Record<string, unknown>
        : {}
      const navigation = Array.isArray(record.navigation) ? record.navigation : []
      const navigationGroups = navigation.map((group): string => {
        if (typeof group === 'object' && group !== null && 'group' in group) {
          const label = (group as Record<string, unknown>).group
          return typeof label === 'string' ? label : '<unnamed>'
        }
        return '<unnamed>'
      })
      return {
        code: asString(record.code) ?? '',
        name: asString(record.name) ?? '',
        version: asString(record.version) ?? '',
        namespace: asString(record.namespace) ?? '',
        minAliothVersion: asString(record.min_alioth_version) ?? '',
        modules: asStringArray(configObj.modules),
        blocks: asStringArray(configObj.blocks),
        routing: {
          base: asString(routingObj.base) ?? '',
          defaultRoute: asString(routingObj.defaultRoute) ?? '',
        },
        navigationGroups,
        roles: {
          defaultRoles: asStringArray(permissions.defaultRoles),
          adminRoles: asStringArray(permissions.adminRoles),
        },
        missing: REQUIRED_FIELDS.filter(key => !(key in record)),
      }
    },
    presentCall: args => ({
      card: 'generic',
      title: `Inspect Alioth app ${args.namespace}/${args.app}`,
      kind: 'other',
      rawInput: { namespace: args.namespace, app: args.app },
    }),
  }))
}
