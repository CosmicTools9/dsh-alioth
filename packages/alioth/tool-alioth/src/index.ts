/**
 * Model-facing Alioth app-artifact tools. Two tools:
 * - `alioth_app_inspect` — read-only validation of an existing `app.json`.
 * - `alioth_app_write` — generate a validated app artifact tree (app.json,
 *   module.json per module, extensions/*.yaml skeletons, Sources/ dirs) under
 *   the configured Pre-Proc root. Write goes through the approval seam when
 *   the deployment composes one (`approvalMode: 'required'`); otherwise the
 *   deployment must choose `'bypass'` explicitly.
 * @module @dsh-alioth/tool-alioth
 */

import { mkdir, readFile, writeFile } from 'node:fs/promises'
import path from 'node:path'
import type { Context } from '@deepseek-ai/cordis'
import z from '@deepseek-ai/schemastery'
import { defineTool } from '@deepseek-ai/dsh-tools'
import { generateApp, generateExtensions, sourceModuleDirs, validateArtifact } from '@dsh-alioth/gen-alioth'
import type {} from '@deepseek-ai/dsh-user-approval'

export const name = 'tool-alioth'
export const inject = ['tools']

/** Deployment choice: the Alioth Pre-Proc artifact tree root (e.g. `<repo>/Pre-Proc`). */
export interface Config {
  preProcRoot: string
  /**
   * Write-approval mode for `alioth_app_write`. `'required'` fails the write
   * without a composed ApprovalService and routes every write through it
   * (grant = `allowed-once`); `'bypass'` writes without asking — choose it
   * only for unattended/CI deployments.
   */
  approvalMode?: 'required' | 'bypass'
}

/** Schemastery configuration for the alioth tool consumer. */
export const Config: z<Config> = z.object({
  preProcRoot: z.string().required(),
  approvalMode: z.union(['required', 'bypass'] as const).default('bypass'),
})

/** Alioth namespace contract: `^[A-Z][a-zA-Z0-9-]*$` (Gateway runtime requirement). */
const NAMESPACE_PATTERN_RE = /^[A-Z][a-zA-Z0-9-]*$/

/** App code is a directory name under `Apps/`: letters, digits, hyphens only. */
const APP_PATTERN_RE = /^[a-zA-Z0-9][a-zA-Z0-9-]*$/

/** Module id lives under `Sources/Modules/` and `config.modules`: same charset as app code. */
const MODULE_PATTERN_RE = /^[a-zA-Z0-9][a-zA-Z0-9-]*$/

/** Fields an Alioth `app.json` must carry; anything else is reported, not rejected. */
const REQUIRED_FIELDS = ['id', 'code', 'namespace', 'name', 'version', 'config'] as const

function asString(value: unknown): string | null {
  return typeof value === 'string' ? value : null
}

function asStringArray(value: unknown): string[] {
  return Array.isArray(value) ? value.filter((item): item is string => typeof item === 'string') : []
}

/**
 * Register the `alioth_app_inspect` and `alioth_app_write` tools on `ctx.tools`.
 * @param ctx - registrant context carrying the tool registry.
 * @param config - deployment's explicit Pre-Proc root and approval posture.
 */
export function apply(ctx: Context, config: Config): void {
  const root = path.resolve(config.preProcRoot)
  const approvalMode = config.approvalMode ?? 'bypass'

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

  ctx.tools.register(defineTool({
    name: 'alioth_app_write',
    description:
      'Generate and persist a validated Alioth app artifact tree: app.json, module.json '
      + 'per module, extensions/*.yaml skeletons, and Sources/Modules/* directories — all '
      + 'under Pre-Proc/{namespace}/Apps/{code}/. Generated app.json/module.json always '
      + 'pass the in-repo contracts before anything is written. Refuses to overwrite an '
      + 'existing app: inspect it first. Requires approval when the deployment sets '
      + 'approvalMode=required.',
    parameters: {
      namespace: {
        type: 'string',
        required: true,
        description: 'Alioth namespace, e.g. "Alioth". Letters, digits, hyphens only.',
      },
      code: {
        type: 'string',
        required: true,
        description: 'New app code (directory name under Apps/). Letters, digits, hyphens only.',
      },
      name: {
        type: 'string',
        required: true,
        description: 'Human-readable app name.',
      },
      modules: {
        type: 'array',
        required: true,
        items: {
          type: 'object',
          additionalProperties: false,
          properties: {
            id: { type: 'string', required: true },
            name: { type: 'string', required: true },
            description: { type: 'string' },
            icon: { type: 'string' },
          },
        },
        description: 'Module specs: id (hyphenated), name, optional description/icon.',
      },
      blocks: {
        type: 'array',
        items: { type: 'string' },
        description: 'Block ids to declare in config.blocks.',
      },
      navigation: {
        type: 'array',
        items: {
          type: 'object',
          additionalProperties: false,
          properties: {
            group: { type: 'string', required: true },
            icon: { type: 'string' },
            modules: { type: 'array', items: { type: 'string' }, required: true },
          },
        },
        description: 'Navigation groups; defaults to one 系统管理 group over all modules.',
      },
      defaultRoles: { type: 'array', items: { type: 'string' } },
      adminRoles: { type: 'array', items: { type: 'string' } },
      version: { type: 'string', description: 'App version; default 0.1.0.' },
      base: { type: 'string', description: 'Routing base; default /apps/{code}.' },
      defaultRoute: { type: 'string', description: 'Default route; default first module.' },
    },
    output: {
      schema: {
        type: 'object',
        additionalProperties: false,
        properties: {
          namespace: { type: 'string', required: true },
          code: { type: 'string', required: true },
          files: { type: 'array', required: true, items: { type: 'string' } },
          moduleIds: { type: 'array', required: true, items: { type: 'string' } },
        },
      },
      render: (_args, value) => [{
        type: 'text',
        text: `Wrote Alioth app ${value.namespace}/${value.code}: ${value.files.length} files, modules ${value.moduleIds.join(', ')}`,
      }],
    },
    async execute(args, exec) {
      if (!NAMESPACE_PATTERN_RE.test(args.namespace)) {
        throw new Error(`alioth_app_write: invalid namespace ${JSON.stringify(args.namespace)} (expected ^[A-Z][a-zA-Z0-9-]*$)`)
      }
      if (!APP_PATTERN_RE.test(args.code)) {
        throw new Error(`alioth_app_write: invalid app code ${JSON.stringify(args.code)} (expected ^[a-zA-Z0-9][a-zA-Z0-9-]*$)`)
      }
      const modules = args.modules.map((module: { id: string; name: string; description?: string; icon?: string }) => {
        if (!MODULE_PATTERN_RE.test(module.id)) {
          throw new Error(`alioth_app_write: invalid module id ${JSON.stringify(module.id)} (expected ^[a-zA-Z0-9][a-zA-Z0-9-]*$)`)
        }
        return module
      })
      const spec = {
        id: String(Date.now()),
        namespace: args.namespace,
        code: args.code,
        name: args.name,
        modules,
        ...(args.version === undefined ? {} : { version: args.version }),
        ...(args.blocks === undefined ? {} : { blocks: args.blocks }),
        ...(args.navigation === undefined ? {} : { navigation: args.navigation }),
        ...(args.defaultRoles === undefined ? {} : { defaultRoles: args.defaultRoles }),
        ...(args.adminRoles === undefined ? {} : { adminRoles: args.adminRoles }),
        ...(args.base === undefined ? {} : { base: args.base }),
        ...(args.defaultRoute === undefined ? {} : { defaultRoute: args.defaultRoute }),
      }
      const generated = generateApp(spec)
      // Contract gate: never persist an artifact that fails its own contract.
      const appValidation = validateArtifact('app', generated.app)
      if (!appValidation.valid) {
        throw new Error(`alioth_app_write: generated app.json fails the app contract: ${appValidation.errors.join('; ')}`)
      }
      for (const module of generated.modules) {
        const validation = validateArtifact('module', module)
        if (!validation.valid) {
          throw new Error(`alioth_app_write: generated module.json for ${String(module.id)} fails the module contract: ${validation.errors.join('; ')}`)
        }
      }

      const appDir = path.resolve(root, args.namespace, 'Apps', args.code)
      if (!appDir.startsWith(root + path.sep)) {
        throw new Error(`alioth_app_write: path escapes preProcRoot: ${appDir}`)
      }
      const existing = await readFile(path.join(appDir, 'app.json'), 'utf8').then(
        () => true,
        () => false,
      )
      if (existing) {
        throw new Error(`alioth_app_write: app ${args.namespace}/${args.code} already exists — inspect it first (alioth_app_inspect)`)
      }

      if (approvalMode === 'required') {
        const approval = ctx.get('approval')
        if (approval === undefined) {
          throw new Error('alioth_app_write: approvalMode=required but no ApprovalService is composed')
        }
        if (exec.agent === undefined) {
          throw new Error('alioth_app_write: approvalMode=required but the call has no agent to route approval')
        }
        const outcome = await approval.request({
          agent: exec.agent,
          toolName: 'alioth_app_write',
          callId: exec.callId,
          reason: `Write Alioth app artifact tree under ${appDir}`,
          signal: exec.signal,
        })
        if (outcome !== 'allowed-once') {
          throw new Error(`alioth_app_write: denied by approval (${outcome})`)
        }
      }

      await mkdir(appDir, { recursive: true })
      const files: string[] = []
      await writeFile(path.join(appDir, 'app.json'), `${JSON.stringify(generated.app, null, 2)}\n`)
      files.push('app.json')
      for (const module of generated.modules) {
        const moduleDir = path.join(appDir, 'modules', String(module.id))
        await mkdir(moduleDir, { recursive: true })
        await writeFile(path.join(moduleDir, 'module.json'), `${JSON.stringify(module, null, 2)}\n`)
        files.push(`modules/${String(module.id)}/module.json`)
      }
      const extensionsDir = path.join(appDir, 'extensions')
      await mkdir(extensionsDir, { recursive: true })
      for (const [file, content] of Object.entries(generateExtensions(args.code))) {
        await writeFile(path.join(extensionsDir, file), content)
        files.push(`extensions/${file}`)
      }
      for (const sourceDir of sourceModuleDirs(modules)) {
        await mkdir(path.join(appDir, sourceDir), { recursive: true })
      }
      return {
        namespace: args.namespace,
        code: args.code,
        files,
        moduleIds: modules.map(module => module.id),
      }
    },
    presentCall: args => ({
      card: 'generic',
      title: `Write Alioth app ${args.namespace}/${args.code}`,
      kind: 'other',
      rawInput: { namespace: args.namespace, code: args.code, modules: args.modules },
    }),
  }))
}
