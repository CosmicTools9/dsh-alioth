/**
 * Model-facing Alioth app-artifact tools. Four tools:
 * - `alioth_app_list` — enumerate apps under a namespace (or all namespaces)
 *   with contract validity per app; the discovery entry for the model.
 * - `alioth_app_inspect` — read-only validation of an existing `app.json`.
 * - `alioth_app_write` — generate a validated app artifact tree (app.json,
 *   module.json per module, extensions/*.yaml skeletons, Sources/ dirs) under
 *   the configured Pre-Proc root. Write goes through the approval seam when
 *   the deployment composes one (`approvalMode: 'required'`); otherwise the
 *   deployment must choose `'bypass'` explicitly.
 * - `alioth_app_configure` — merge enrichment fields AND grow the app
 *   (add modules/blocks) programmatically.
 * @module @dsh-alioth/tool-alioth
 */

import { existsSync } from 'node:fs'
import { mkdir, readFile, readdir, rm, writeFile } from 'node:fs/promises'
import path from 'node:path'
import type { Context } from '@deepseek-ai/cordis'
import z from '@deepseek-ai/schemastery'
import { defineTool, type ToolRunContext } from '@deepseek-ai/dsh-tools'
import { generateApp, generateExtensions, generateModule, generateNamespaceWorkspace, generateService, generateServiceCrate, sourceModuleDirs, validateArtifact } from '@dsh-alioth/gen-alioth'
import { validateCoordinates } from '@dsh-alioth/skill-alioth'
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

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

/** Version used for newly grown modules when the existing app.json lacks one. */
const DEFAULT_VERSION = '0.1.0'

/**
 * Route one tool call through the composed ApprovalService when the deployment
 * chose `approvalMode: 'required'`. Grants are `allowed-once`; anything else
 * fails the call. Shared by every destructive/persisting tool.
 */
async function requestApproval(
  ctx: Context,
  exec: ToolRunContext,
  toolName: string,
  reason: string,
): Promise<void> {
  const approval = ctx.get('approval')
  if (approval === undefined) {
    throw new Error(`${toolName}: approvalMode=required but no ApprovalService is composed`)
  }
  if (exec.agent === undefined) {
    throw new Error(`${toolName}: approvalMode=required but the call has no agent to route approval`)
  }
  const outcome = await approval.request({
    agent: exec.agent,
    toolName,
    callId: exec.callId,
    reason,
    signal: exec.signal,
  })
  if (outcome !== 'allowed-once') {
    throw new Error(`${toolName}: denied by approval (${outcome})`)
  }
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
    name: 'alioth_app_list',
    description:
      'Enumerate Alioth apps under the configured Pre-Proc tree. With `namespace`, lists the '
      + 'apps of that namespace; without it, lists every namespace and its apps. Each app '
      + 'entry carries code, name, version, status, module ids, and contract validity — '
      + 'invalid artifacts are flagged, not hidden. Use this to discover existing apps '
      + 'before creating or extending one; never guess an app code.',
    parameters: {
      namespace: {
        type: 'string',
        description: 'Optional namespace filter — resolve with alioth_workspace_current first; omit = all namespaces. Letters, digits, hyphens only.',
      },
    },
    output: {
      schema: {
        type: 'object',
        additionalProperties: false,
        properties: {
          namespaces: {
            type: 'array',
            required: true,
            items: {
              type: 'object',
              additionalProperties: false,
              properties: {
                namespace: { type: 'string', required: true },
                apps: {
                  type: 'array',
                  required: true,
                  items: {
                    type: 'object',
                    additionalProperties: false,
                    properties: {
                      code: { type: 'string', required: true },
                      name: { type: 'string', required: true },
                      version: { type: 'string', required: true },
                      status: { type: 'string', required: true },
                      modules: { type: 'array', required: true, items: { type: 'string' } },
                      valid: { type: 'boolean', required: true },
                      missing: { type: 'array', required: true, items: { type: 'string' } },
                    },
                  },
                },
              },
            },
          },
        },
      },
      render: (_args, value) => [{
        type: 'text',
        text: value.namespaces
          .map((ns: { namespace: string; apps: Array<{ code: string; valid: boolean }> }) =>
            `${ns.namespace}: ${ns.apps.length} app(s) — ${ns.apps.map(app => `${app.code}${app.valid ? '' : ' (invalid)'}`).join(', ') || 'none'}`)
          .join('\n'),
      }],
    },
    async execute(args) {
      if (args.namespace !== undefined && !NAMESPACE_PATTERN_RE.test(args.namespace)) {
        throw new Error(`alioth_app_list: invalid namespace ${JSON.stringify(args.namespace)} (expected ^[A-Z][a-zA-Z0-9-]*$)`)
      }
      const namespaceDirs = args.namespace === undefined
        ? await readdir(root, { withFileTypes: true }).then(entries =>
          entries.filter(entry => entry.isDirectory() && !entry.name.startsWith('.')).map(entry => entry.name)).catch(() => [])
        : [args.namespace]
      const namespaces: Array<{
        namespace: string
        apps: Array<{
          code: string
          name: string
          version: string
          status: string
          modules: string[]
          valid: boolean
          missing: string[]
        }>
      }> = []
      for (const namespace of namespaceDirs) {
        const appsRoot = path.join(root, namespace, 'Apps')
        const appDirs = await readdir(appsRoot, { withFileTypes: true }).then(entries =>
          entries.filter(entry => entry.isDirectory() && !entry.name.startsWith('.')).map(entry => entry.name)).catch(() => [])
        const apps: Array<{
          code: string
          name: string
          version: string
          status: string
          modules: string[]
          valid: boolean
          missing: string[]
        }> = []
        for (const app of appDirs) {
          const appFile = path.join(appsRoot, app, 'app.json')
          let parsed: unknown
          let parseError: string | null = null
          try {
            parsed = JSON.parse(await readFile(appFile, 'utf8'))
          } catch (error) {
            parseError = error instanceof Error ? error.message : String(error)
          }
          if (parseError !== null || typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) {
            apps.push({
              code: app,
              name: '',
              version: '',
              status: '',
              modules: [],
              valid: false,
              missing: REQUIRED_FIELDS.slice(),
            })
            continue
          }
          const record = parsed as Record<string, unknown>
          const configObj = typeof record.config === 'object' && record.config !== null
            ? record.config as Record<string, unknown>
            : {}
          const validation = validateArtifact('app', record)
          apps.push({
            code: asString(record.code) ?? app,
            name: asString(record.name) ?? '',
            version: asString(record.version) ?? '',
            status: asString(record.status) ?? '',
            modules: asStringArray(configObj.modules),
            valid: validation.valid,
            missing: validation.valid ? [] : REQUIRED_FIELDS.filter(key => !(key in record)),
          })
        }
        apps.sort((a, b) => String(a.code).localeCompare(String(b.code)))
        namespaces.push({ namespace, apps })
      }
      namespaces.sort((a, b) => String(a.namespace).localeCompare(String(b.namespace)))
      return { namespaces }
    },
    presentCall: args => ({
      card: 'generic',
      title: args.namespace === undefined ? 'List Alioth apps' : `List Alioth apps in ${args.namespace}`,
      kind: 'other',
      rawInput: args.namespace === undefined ? {} : { namespace: args.namespace },
    }),
  }))

  ctx.tools.register(defineTool({
    name: 'alioth_app_inspect',
    description:
      'Inspect an Alioth app artifact (app.json) under the configured Pre-Proc tree. '
      + 'Returns the app code, version, namespace, status, description, brand, goal, '
      + 'non-scope, required modules and blocks, routing, navigation groups, roles, and '
      + 'any missing required fields — the full readback of everything app_write and '
      + 'app_configure can set. Use this before creating or extending an Alioth app so '
      + 'the model reads the real artifact — never guess an app\'s contents.',
    parameters: {
      namespace: {
        type: 'string',
        required: true,
        description: 'The caller\'s own workspace namespace — resolve with alioth_workspace_current first. Letters, digits, hyphens only.',
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
          status: { type: 'string', required: true },
          description: { type: 'string', required: true },
          goal: { type: 'string', required: true },
          nonScope: { type: 'array', required: true, items: { type: 'string' } },
          brand: {
            type: 'object', required: true, additionalProperties: false,
            properties: {
              primary: { type: 'string' },
              logo: { type: 'string' },
            },
          },
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
      const brand = typeof record.brand === 'object' && record.brand !== null
        ? record.brand as Record<string, unknown>
        : {}
      return {
        code: asString(record.code) ?? '',
        name: asString(record.name) ?? '',
        version: asString(record.version) ?? '',
        namespace: asString(record.namespace) ?? '',
        minAliothVersion: asString(record.min_alioth_version) ?? '',
        status: asString(record.status) ?? '',
        description: asString(record.description) ?? '',
        goal: asString(record.goal) ?? '',
        nonScope: asStringArray(record.non_scope),
        brand: {
          primary: asString(brand.primary) ?? '',
          logo: asString(brand.logo) ?? '',
        },
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
        description: 'The caller\'s own workspace namespace — resolve with alioth_workspace_current first. Letters, digits, hyphens only.',
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
      description: {
        type: 'string',
        description: 'Optional one-line app description (contract-declared field).',
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
      brand: {
        type: 'object', additionalProperties: false,
        properties: {
          primary: { type: 'string', description: 'Primary brand color (hex).' },
          logo: { type: 'string', description: 'Logo asset path.' },
        },
      },
      goal: { type: 'string', description: 'App goal (17-field alignment).' },
      nonScope: { type: 'array', items: { type: 'string' }, description: 'Explicit non-scope statements.' },
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
        ...(args.description === undefined ? {} : { description: args.description }),
        ...(args.version === undefined ? {} : { version: args.version }),
        ...(args.blocks === undefined ? {} : { blocks: args.blocks }),
        ...(args.navigation === undefined ? {} : { navigation: args.navigation }),
        ...(args.defaultRoles === undefined ? {} : { defaultRoles: args.defaultRoles }),
        ...(args.adminRoles === undefined ? {} : { adminRoles: args.adminRoles }),
        ...(args.base === undefined ? {} : { base: args.base }),
        ...(args.defaultRoute === undefined ? {} : { defaultRoute: args.defaultRoute }),
        ...(args.brand === undefined ? {} : { brand: args.brand }),
        ...(args.goal === undefined ? {} : { goal: args.goal }),
        ...(args.nonScope === undefined ? {} : { nonScope: args.nonScope }),
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
        await requestApproval(ctx, exec, 'alioth_app_write', `Write Alioth app artifact tree under ${appDir}`)
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
      // Namespace-level Sources mirror (fb28b5e02): Sources/Apps/Modules/{id}
      // lives under the namespace, not inside the app dir.
      for (const sourceDir of sourceModuleDirs(modules)) {
        await mkdir(path.resolve(root, args.namespace, sourceDir), { recursive: true })
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

  ctx.tools.register(defineTool({
    name: 'alioth_app_configure',
    description:
      'Programmatic app enrichment and growth: merge brand / navigation / routing / '
      + 'permissions / goal / non_scope into an existing app.json AND add modules (each new '
      + 'module gets a contract-valid module.json plus a Sources/Modules dir and joins '
      + 'config.modules and navigation) or replace blocks. Contract-validates before write. '
      + 'Deterministic — no LLM generation; the model supplies structured parameters only. '
      + 'Idempotent: fields not provided are left untouched; provided fields replace; '
      + 're-adding an existing module id is a no-op. Refuses to write when the merged '
      + 'app.json fails its contract. Use this instead of writing app.json by hand.',
    parameters: {
      namespace: { type: 'string', required: true },
      app: { type: 'string', required: true },
      modules: {
        type: 'array',
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
        description: 'New module specs to add to the app (existing ids are no-ops).',
      },
      blocks: {
        type: 'array',
        items: { type: 'string' },
        description: 'Replaces config.blocks wholesale.',
      },
      brand: {
        type: 'object', additionalProperties: false,
        properties: {
          primary: { type: 'string' },
          logo: { type: 'string' },
        },
      },
      navigation: {
        type: 'array',
        items: {
          type: 'object', additionalProperties: false,
          properties: {
            group: { type: 'string', required: true },
            icon: { type: 'string' },
            modules: { type: 'array', items: { type: 'string' }, required: true },
          },
        },
      },
      defaultRoles: { type: 'array', items: { type: 'string' } },
      adminRoles: { type: 'array', items: { type: 'string' } },
      base: { type: 'string' },
      defaultRoute: { type: 'string' },
      status: { type: 'string', description: 'Lifecycle status, e.g. "developing" or "archived" (contract-declared field).' },
      goal: { type: 'string' },
      nonScope: { type: 'array', items: { type: 'string' } },
    },
    output: {
      schema: {
        type: 'object',
        additionalProperties: false,
        properties: {
          namespace: { type: 'string', required: true },
          app: { type: 'string', required: true },
          updated: { type: 'array', required: true, items: { type: 'string' } },
          file: { type: 'string', required: true },
        },
      },
      render: (_args, value) => [{
        type: 'text',
        text: `Configured ${value.namespace}/${value.app}: updated ${value.updated.join(', ')}`,
      }],
    },
    async execute(args, _exec) {
      const root = path.resolve(config.preProcRoot)
      const appFile = path.resolve(root, args.namespace, 'Apps', args.app, 'app.json')
      if (!appFile.startsWith(root + path.sep)) {
        throw new Error(`alioth_app_configure: path escapes preProcRoot: ${appFile}`)
      }
      const existing = await readFile(appFile, 'utf8').catch(() => undefined)
      if (existing === undefined) {
        throw new Error(`alioth_app_configure: no app.json at ${appFile} — create the app first (alioth_app_write)`)
      }
      const app = JSON.parse(existing) as Record<string, unknown>
      const updated: string[] = []
      if (args.brand !== undefined) {
        const current = typeof app.brand === 'object' && app.brand !== null ? app.brand as Record<string, unknown> : {}
        const merged = { ...current }
        for (const [key, value] of Object.entries(args.brand)) {
          if (value !== undefined) { merged[key] = value; updated.push(`brand.${key}`) }
        }
        if (Object.keys(merged).length > 0) { app.brand = merged }
      }
      if (args.navigation !== undefined) {
        app.navigation = args.navigation as unknown
        updated.push('navigation')
      }
      if (args.defaultRoles !== undefined || args.adminRoles !== undefined) {
        const permissions = typeof app.permissions === 'object' && app.permissions !== null
          ? app.permissions as Record<string, unknown> : {}
        if (args.defaultRoles !== undefined) { permissions.defaultRoles = args.defaultRoles; updated.push('permissions.defaultRoles') }
        if (args.adminRoles !== undefined) { permissions.adminRoles = args.adminRoles; updated.push('permissions.adminRoles') }
        app.permissions = permissions
      }
      if (args.base !== undefined || args.defaultRoute !== undefined) {
        const routing = typeof app.routing === 'object' && app.routing !== null ? app.routing as Record<string, unknown> : {}
        if (args.base !== undefined) { routing.base = args.base; updated.push('routing.base') }
        if (args.defaultRoute !== undefined) { routing.defaultRoute = args.defaultRoute; updated.push('routing.defaultRoute') }
        app.routing = routing
      }
      if (args.goal !== undefined) { app.goal = args.goal; updated.push('goal') }
      if (args.status !== undefined) { app.status = args.status; updated.push('status') }
      if (args.nonScope !== undefined) { app.non_scope = args.nonScope; updated.push('non_scope') }
      if (args.blocks !== undefined) {
        const config = typeof app.config === 'object' && app.config !== null
          ? app.config as Record<string, unknown>
          : {}
        config.blocks = [...args.blocks]
        app.config = config
        updated.push('config.blocks')
      }
      if (args.modules !== undefined) {
        const config = typeof app.config === 'object' && app.config !== null
          ? app.config as Record<string, unknown>
          : {}
        const moduleIds = asStringArray(config.modules)
        const ownerVersion = typeof app.version === 'string' ? app.version : DEFAULT_VERSION
        const ownerNamespace = typeof app.namespace === 'string' ? app.namespace : args.namespace
        const newModules: Array<{ id: string; name: string; description?: string; icon?: string }> = []
        for (const module of args.modules) {
          if (!MODULE_PATTERN_RE.test(module.id)) {
            throw new Error(`alioth_app_configure: invalid module id ${JSON.stringify(module.id)} (expected ^[a-zA-Z0-9][a-zA-Z0-9-]*$)`)
          }
          if (!moduleIds.includes(module.id)) {
            moduleIds.push(module.id)
            newModules.push(module)
          }
        }
        if (newModules.length > 0) {
          config.modules = moduleIds
          app.config = config
          const appDir = path.dirname(appFile)
          for (const module of newModules) {
            const generated = generateModule({ namespace: ownerNamespace, version: ownerVersion }, module)
            const validation = validateArtifact('module', generated)
            if (!validation.valid) {
              throw new Error(`alioth_app_configure: generated module.json for ${module.id} fails the module contract: ${validation.errors.join('; ')}`)
            }
            await mkdir(path.join(appDir, 'modules', module.id), { recursive: true })
            await writeFile(path.join(appDir, 'modules', module.id, 'module.json'), `${JSON.stringify(generated, null, 2)}\n`)
            await mkdir(path.join(appDir, 'Sources', 'Modules', module.id), { recursive: true })
            updated.push(`modules.${module.id}`)
          }
          // Every module must be reachable: append new ids to the 系统管理 group when
          // present, else the first group; a navigation-less app gets the default group.
          const navigation = Array.isArray(app.navigation) ? app.navigation : null
          if (navigation === null || navigation.length === 0) {
            app.navigation = [{ group: '系统管理', icon: 'Settings', modules: moduleIds }]
          } else {
            const target = (navigation.find(group => isRecord(group) && group.group === '系统管理') ?? navigation[0])
            if (isRecord(target)) {
              const existing = asStringArray(target.modules)
              for (const id of newModules.map(module => module.id)) {
                if (!existing.includes(id)) { existing.push(id) }
              }
              target.modules = existing
            }
            app.navigation = navigation
          }
          updated.push('navigation')
        }
      }

      // Contract gate: never persist an artifact that fails its own contract.
      const validation = validateArtifact('app', app)
      if (!validation.valid) {
        throw new Error(`alioth_app_configure: merged app.json fails the app contract: ${validation.errors.join('; ')}`)
      }
      if (updated.length === 0) {
        return { namespace: args.namespace, app: args.app, updated: [], file: appFile }
      }
      await writeFile(appFile, `${JSON.stringify(app, null, 2)}\n`, 'utf8')
      return { namespace: args.namespace, app: args.app, updated, file: appFile }
    },
    presentCall: args => ({
      card: 'generic',
      title: `Configure Alioth app ${args.namespace}/${args.app}`,
      kind: 'other',
      rawInput: { namespace: args.namespace, app: args.app },
    }),
  }))

  ctx.tools.register(defineTool({
    name: 'alioth_app_delete',
    description:
      'Permanently delete an Alioth app artifact tree (app.json, modules, extensions, '
      + 'Sources) under Pre-Proc/{namespace}/Apps/{app}/. Destructive and irreversible: '
      + 'requires confirm: true, fails when the app does not exist, and routes through the '
      + 'approval seam when the deployment sets approvalMode=required. List and inspect '
      + 'before deleting; prefer alioth_app_configure status=archived for soft retirement.',
    parameters: {
      namespace: {
        type: 'string',
        required: true,
        description: 'The caller\'s own workspace namespace — resolve with alioth_workspace_current first. Letters, digits, hyphens only.',
      },
      app: {
        type: 'string',
        required: true,
        description: 'App code as in the directory name under Apps/. Letters, digits, hyphens only.',
      },
      confirm: {
        type: 'boolean',
        description: 'Must be true; the delete is irreversible.',
      },
    },
    output: {
      schema: {
        type: 'object',
        additionalProperties: false,
        properties: {
          namespace: { type: 'string', required: true },
          app: { type: 'string', required: true },
          files: { type: 'array', required: true, items: { type: 'string' } },
        },
      },
      render: (_args, value) => [{
        type: 'text',
        text: `Deleted Alioth app ${value.namespace}/${value.app}: ${value.files.length} paths removed`,
      }],
    },
    async execute(args, exec) {
      if (!NAMESPACE_PATTERN_RE.test(args.namespace)) {
        throw new Error(`alioth_app_delete: invalid namespace ${JSON.stringify(args.namespace)} (expected ^[A-Z][a-zA-Z0-9-]*$)`)
      }
      if (!APP_PATTERN_RE.test(args.app)) {
        throw new Error(`alioth_app_delete: invalid app code ${JSON.stringify(args.app)} (expected ^[a-zA-Z0-9][a-zA-Z0-9-]*$)`)
      }
      const appDir = path.resolve(root, args.namespace, 'Apps', args.app)
      if (!appDir.startsWith(root + path.sep)) {
        throw new Error(`alioth_app_delete: path escapes preProcRoot: ${appDir}`)
      }
      const appFile = path.join(appDir, 'app.json')
      const existing = await readFile(appFile, 'utf8').then(
        () => true,
        () => false,
      )
      if (!existing) {
        throw new Error(`alioth_app_delete: no app.json at ${appFile}`)
      }
      if (args.confirm !== true) {
        throw new Error('alioth_app_delete: destructive — pass confirm: true to delete the app tree')
      }
      if (approvalMode === 'required') {
        await requestApproval(ctx, exec, 'alioth_app_delete', `Delete Alioth app artifact tree under ${appDir}`)
      }
      const files = await readdir(appDir, { recursive: true }).catch(() => [])
      await rm(appDir, { recursive: true, force: true })
      return { namespace: args.namespace, app: args.app, files }
    },
    presentCall: args => ({
      card: 'generic',
      title: `Delete Alioth app ${args.namespace}/${args.app}`,
      kind: 'other',
      rawInput: { namespace: args.namespace, app: args.app },
    }),
  }))

  ctx.tools.register(defineTool({
    name: 'alioth_sources_scaffold',
    description:
      'Deterministic backend Sources scaffold (mirror layout): per declared service writes '
      + 'Sources/Apps/Services/{id}/service.json (contract-validated; entities with coordinates '
      + 'from your semantic alignment) plus a mount-only backend crate shell (Cargo.toml + '
      + 'src/lib.rs), and the namespace workspace Sources/Cargo.toml joining the crates. '
      + 'Business/DTO code is NOT generated here — author it in gated workflow steps '
      + '(alioth-service track); the shell only mounts the service scope. Refuses overwrite: '
      + 'an existing service.json or namespace Cargo.toml stops the scaffold (inspect first). '
      + 'Compiles only where the Framework crates resolve (AliothStudio checkout or provisioned '
      + 'content root).',
    parameters: {
      namespace: {
        type: 'string',
        required: true,
        description: 'The caller\'s own workspace namespace — resolve with alioth_workspace_current first.',
      },
      services: {
        type: 'array',
        required: true,
        items: {
          type: 'object',
          additionalProperties: false,
          properties: {
            id: { type: 'string', required: true },
            domain: { type: 'string', required: true },
            layer: { type: 'number', required: true },
            dtoDependencies: { type: 'array', items: { type: 'string' } },
            entities: {
              type: 'array',
              required: true,
              items: {
                type: 'object',
                additionalProperties: false,
                properties: {
                  name: { type: 'string', required: true },
                  table: { type: 'string', required: true },
                  inherits: { type: 'string', required: true },
                  coordinates: {
                    type: 'object',
                    additionalProperties: false,
                    properties: {
                      scene: { type: 'string', required: true },
                      factor: { type: 'string', required: true },
                      function: { type: 'string', required: true },
                    },
                  },
                },
              },
            },
          },
        },
        description: 'Service declarations (one service per ontology domain group).',
      },
    },
    output: {
      schema: {
        type: 'object',
        additionalProperties: false,
        properties: {
          namespace: { type: 'string', required: true },
          serviceIds: { type: 'array', required: true, items: { type: 'string' } },
          files: { type: 'array', required: true, items: { type: 'string' } },
          summary: { type: 'string', required: true },
        },
      },
      render: (_args, value) => [{
        type: 'text',
        text: `Scaffolded ${String(value.namespace)} Sources: ${value.serviceIds.length} service(s), `
          + `${value.files.length} files (contract shells + workspace manifest)`,
      }],
    },
    async execute(args) {
      if (args.services.length === 0) {
        throw new Error('alioth_sources_scaffold: declare at least one service')
      }
      const specs = args.services.map(service => generateService({
        id: service.id,
        namespace: args.namespace,
        domain: service.domain,
        services: [],
        layer: service.layer,
        dtoDependencies: service.dtoDependencies ?? [],
        backendCrate: `alioth-service-${service.id}`,
        hasBackend: true,
        hasFrontend: false,
        ontology: {
          entities: (service.entities ?? []).map(entity => ({
            name: entity.name,
            table: entity.table,
            inherits: entity.inherits,
            ...(entity.coordinates === undefined ? {} : { coordinates: entity.coordinates }),
          })),
        },
      }))
      // G3.5 discipline, deterministically enforced: declared coordinates MUST
      // be real dictionary codes; omitted coordinates stay the honest Unclear
      // state (never guess, never accept placeholder codes).
      for (const service of args.services) {
        for (const entity of service.entities ?? []) {
          const issues = validateCoordinates(entity.coordinates)
          if (issues.length > 0) {
            throw new Error(`alioth_sources_scaffold: entity ${entity.name} (service ${service.id}) coordinate check failed — align via alioth_schema_semantic_search or omit the coordinates (Unclear): ${issues.map(issue => issue.message).join('; ')}`)
          }
        }
      }
      for (const [index, service] of specs.entries()) {
        const validation = validateArtifact('service', service)
        if (!validation.valid) {
          throw new Error(`alioth_sources_scaffold: service ${args.services[index]?.id ?? index} fails contract: ${validation.errors.join('; ')}`)
        }
      }

      const nsRoot = path.resolve(root, args.namespace)
      const sourcesRoot = path.join(nsRoot, 'Sources')
      const written: string[] = []
      const refused: string[] = []

      const workspaceManifest = path.join(sourcesRoot, 'Cargo.toml')
      if (existsSync(workspaceManifest)) {
        refused.push('Sources/Cargo.toml')
      }
      const planned: { readonly relative: string; readonly content: string }[] = []
      for (const [serviceIndex, service] of args.services.entries()) {
        const serviceDir = path.join(sourcesRoot, 'Apps', 'Services', service.id)
        const serviceJson = path.join(serviceDir, 'service.json')
        if (existsSync(serviceJson)) {
          refused.push(`Sources/Apps/Services/${service.id}/service.json`)
          continue
        }
        planned.push({ relative: `Sources/Apps/Services/${service.id}/service.json`, content: `${JSON.stringify(specs[serviceIndex], null, 2)}\n` })
        for (const [relative, content] of Object.entries(generateServiceCrate(args.namespace, service.id))) {
          planned.push({ relative: `Sources/Apps/Services/${service.id}/${relative}`, content })
        }
      }
      if (refused.length > 0 && planned.length > 0) {
        throw new Error(`alioth_sources_scaffold: refusing partial scaffold — existing: ${refused.join(', ')} (inspect first, scaffold the rest separately)`)
      }
      if (refused.length > 0) {
        throw new Error(`alioth_sources_scaffold: nothing to scaffold — existing: ${refused.join(', ')}`)
      }
      if (existsSync(workspaceManifest)) {
        // All services exist; only the manifest is present — nothing to do.
        return {
          namespace: args.namespace,
          serviceIds: [],
          files: [],
          summary: 'Sources already scaffolded (workspace manifest present; every declared service exists)',
        }
      }
      planned.push({ relative: 'Sources/Cargo.toml', content: generateNamespaceWorkspace(args.namespace, args.services.map(service => service.id)) })
      for (const file of planned) {
        const target = path.join(nsRoot, file.relative)
        if (!target.startsWith(nsRoot + path.sep)) {
          throw new Error(`alioth_sources_scaffold: path escapes namespace root: ${file.relative}`)
        }
        await mkdir(path.dirname(target), { recursive: true })
        await writeFile(target, file.content)
        written.push(file.relative)
      }
      return {
        namespace: args.namespace,
        serviceIds: args.services.map(service => service.id),
        files: written,
        summary: `scaffolded ${args.services.length} service(s): contract service.json + mount-only crate shells; author DTO/business code in gated workflow steps (alioth-service track)`,
      }
    },
    presentCall: args => ({
      card: 'generic',
      title: `Scaffold Sources ${args.namespace} (${args.services.length} services)`,
      kind: 'other',
      rawInput: args as Record<string, unknown>,
    }),
  }))
}
