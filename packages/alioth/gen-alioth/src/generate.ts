/**
 * Pure artifact generators: `app.json` + per-module `module.json` from a
 * structural spec. No I/O — callers own persistence. Generated output always
 * validates against the contracts in `contracts/`.
 * @module @dsh-alioth/gen-alioth/generate
 */

export interface ModuleSpec {
  readonly id: string
  readonly name: string
  readonly description?: string
  readonly icon?: string
}

export interface NavigationGroup {
  readonly group: string
  readonly icon?: string
  readonly modules: readonly string[]
}

export interface AppSpec {
  /** Application id (zuid-style snowflake; caller-owned). */
  readonly id: string
  readonly namespace: string
  readonly code: string
  readonly name: string
  readonly version?: string
  readonly modules: readonly ModuleSpec[]
  readonly blocks?: readonly string[]
  readonly navigation?: readonly NavigationGroup[]
  readonly defaultRoles?: readonly string[]
  readonly adminRoles?: readonly string[]
  /** Routing base; defaults to `/apps/{code}`. */
  readonly base?: string
  /** Default route; defaults to the first module id. */
  readonly defaultRoute?: string
}

export interface GeneratedApp {
  readonly app: Record<string, unknown>
  readonly modules: readonly Record<string, unknown>[]
}

const DEFAULT_VERSION = '0.1.0'
const DEFAULT_MIN_ALIOTH_VERSION = '10.0.0'

/** The app-level extensions per the distribution's artifact tree (DESIGN_INTENT). */
export const EXTENSION_FILES = ['constraints', 'rules', 'statemachines', 'workflows'] as const

/** Skeleton YAML for one extension kind. Honest placeholder: empty doc + provenance. */
export function generateExtension(kind: string, code: string): string {
  return `# dsh-alioth generated skeleton — ${kind} for app ${code}.\n# Shape follows the Alioth model extensions/*.yaml contract; extend before import.\n{}\n`
}

/** Extension file names → skeleton content for an app. */
export function generateExtensions(code: string): Readonly<Record<string, string>> {
  return Object.fromEntries(EXTENSION_FILES.map(kind => [`${kind}.yaml`, generateExtension(kind, code)]))
}

/** Source-skeleton directories for an app (modules; services come with service.json generation). */
export function sourceModuleDirs(modules: readonly ModuleSpec[]): readonly string[] {
  return modules.map(module => `Sources/Modules/${module.id}`)
}

/** Build the app.json object plus one module.json per module. */
export function generateApp(spec: AppSpec): GeneratedApp {
  const version = spec.version ?? DEFAULT_VERSION
  const moduleIds = spec.modules.map(module => module.id)
  const base = spec.base ?? `/apps/${spec.code}`
  const defaultRoute = spec.defaultRoute
    ?? (moduleIds.length === 0 ? '/' : `/${moduleIds[0]}`)
  const navigation = spec.navigation === undefined
    ? [{ group: '系统管理', icon: 'Settings', modules: moduleIds }]
    : spec.navigation.map(group => ({
      group: group.group,
      icon: group.icon ?? 'AppstoreOutlined',
      modules: [...group.modules],
    }))
  const app = {
    id: spec.id,
    code: spec.code,
    namespace: spec.namespace,
    name: spec.name,
    version,
    config: {
      modules: moduleIds,
      blocks: [...(spec.blocks ?? [])],
    },
    permissions: {
      defaultRoles: [...(spec.defaultRoles ?? ['admin', 'user'])],
      adminRoles: [...(spec.adminRoles ?? ['admin'])],
    },
    routing: { base, defaultRoute },
    navigation,
    min_alioth_version: DEFAULT_MIN_ALIOTH_VERSION,
  }
  const modules = spec.modules.map(module => ({
    id: module.id,
    namespace: spec.namespace,
    name: module.name,
    category: 'app',
    status: 'draft',
    routePrefix: `/${module.id}`,
    icon: module.icon ?? 'AppstoreOutlined',
    hasBackend: false,
    hasFrontend: true,
    version,
    selectable: true,
    description: module.description ?? '',
  }))
  return { app, modules }
}
