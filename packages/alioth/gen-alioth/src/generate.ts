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
  /** Optional human-readable one-liner (contract-declared field). */
  readonly description?: string
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
  /** Brand: primary color + logo asset. */
  readonly brand?: { readonly primary?: string; readonly logo?: string }
  /** App goal (17-field alignment; free-form intent). */
  readonly goal?: string
  /** Explicit non-scope statements (model wire shape: string[]). */
  readonly nonScope?: readonly string[]
}

export interface GeneratedApp {
  readonly app: Record<string, unknown>
  readonly modules: readonly Record<string, unknown>[]
}

const DEFAULT_VERSION = '0.1.0'
const DEFAULT_MIN_ALIOTH_VERSION = '10.0.0'

/** The app-level extensions per the distribution's artifact tree (DESIGN_INTENT). */
export const EXTENSION_FILES = ['constraints', 'rules', 'statemachines', 'workflows'] as const

/** Skeleton YAML for one extension kind. Honest placeholder: empty doc + provenance.
 *  Top-level MUST be a YAML sequence: constraints/rules/statemachines/workflows
 *  deserialize into `Vec<T>` at the Gateway ExtensionLoader (2026-08-23 contract
 *  fix — `constraints: []` map wrappers crash Gateway startup). */
export function generateExtension(kind: string, code: string): string {
  return `# dsh-alioth generated skeleton — ${kind} for app ${code}.\n# Shape follows the Alioth model extensions/*.yaml contract; extend before import.\n[]\n`
}

/** Extension file names → skeleton content for an app. */
export function generateExtensions(code: string): Readonly<Record<string, string>> {
  return Object.fromEntries(EXTENSION_FILES.map(kind => [`${kind}.yaml`, generateExtension(kind, code)]))
}

/** Source-skeleton directories for an app (modules; services come with service.json generation).
 *  Mirror layout (fb28b5e02): everything lives under Sources/Apps/. */
export function sourceModuleDirs(modules: readonly ModuleSpec[]): readonly string[] {
  return modules.map(module => `Sources/Apps/Modules/${module.id}`)
}

/**
 * Build one module.json for an owner app. Shared by `generateApp` (all
 * modules at creation) and app-growth paths (one module at a time); the
 * module version follows the owning app's version.
 * @param owner - namespace + version of the owning app.json.
 * @param spec - module spec (id, name, optional description/icon).
 */
export function generateModule(
  owner: { readonly namespace: string; readonly version: string },
  spec: ModuleSpec,
): Record<string, unknown> {
  return {
    id: spec.id,
    namespace: owner.namespace,
    name: spec.name,
    category: 'business',
    status: 'planned',
    routePrefix: `/${spec.id}`,
    icon: spec.icon ?? 'AppstoreOutlined',
    hasBackend: false,
    hasFrontend: true,
    version: owner.version,
    selectable: true,
    description: spec.description ?? '',
  }
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
  const brand = spec.brand === undefined ? undefined
    : Object.fromEntries(Object.entries(spec.brand).filter(([, value]) => value !== undefined))
  const app = {
    id: spec.id,
    code: spec.code,
    namespace: spec.namespace,
    name: spec.name,
    version,
    ...(spec.description === undefined ? {} : { description: spec.description }),
    ...(brand === undefined || Object.keys(brand).length === 0 ? {} : { brand }),
    ...(spec.goal === undefined ? {} : { goal: spec.goal }),
    ...(spec.nonScope === undefined ? {} : { non_scope: [...spec.nonScope] }),
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
  const modules = spec.modules.map(module => generateModule({ namespace: spec.namespace, version }, module))
  return { app, modules }
}

// ── service.json generator ───────────────────────────────────────────────

/** One entity's ontology mapping for a service. */
export interface ServiceEntitySpec {
  readonly name: string
  readonly table: string
  readonly inherits: string
  readonly coordinates?: { readonly scene: string; readonly factor: string; readonly function: string }
  /** Field mappings: json_path = field name; column = physical isahl column (reference localKey). */
  readonly fieldMappings?: readonly { readonly jsonPath: string; readonly column: string; readonly scalar?: string }[]
  readonly relationships?: readonly { readonly target: string; readonly type: string; readonly via: string }[]
}

/** The service.json artifact (contract: `service`). */
export interface ServiceSpec {
  readonly id: string
  readonly domain: string
  readonly services: readonly string[]
  readonly layer: number
  readonly dtoDependencies: readonly string[]
  readonly backendCrate: string
  readonly hasBackend: boolean
  readonly hasFrontend: boolean
  readonly version?: string
  readonly ontology: { readonly entities: readonly ServiceEntitySpec[] }
}

/** Build a service.json from an ontology spec; always passes the service contract. */
export function generateService(spec: ServiceSpec): Record<string, unknown> {
  return {
    id: spec.id,
    domain: spec.domain,
    services: [...spec.services],
    layer: spec.layer,
    dtoDependencies: [...spec.dtoDependencies],
    backendCrate: spec.backendCrate,
    hasBackend: spec.hasBackend,
    hasFrontend: spec.hasFrontend,
    version: spec.version ?? DEFAULT_VERSION,
    ontology: {
      entities: spec.ontology.entities.map(entity => ({
        name: entity.name,
        table: entity.table,
        inherits: entity.inherits,
        ...(entity.coordinates === undefined ? {} : { coordinates: entity.coordinates }),
        ...(entity.fieldMappings === undefined || entity.fieldMappings.length === 0
          ? {}
          : {
            field_mappings: entity.fieldMappings.map(mapping => ({
              json_path: mapping.jsonPath,
              column: mapping.column,
              ...(mapping.scalar === undefined ? {} : { scalar: mapping.scalar }),
            })),
          }),
        ...(entity.relationships === undefined || entity.relationships.length === 0
          ? {}
          : { relationships: entity.relationships.map(relationship => ({ ...relationship })) }),
      })),
    },
  }
}

/** Source-skeleton directories for services (mirror layout: Sources/Apps/Services). */
export function sourceServiceDirs(services: readonly { readonly id: string }[]): readonly string[] {
  return services.map(service => `Sources/Apps/Services/${service.id}`)
}

// ── Sources scaffold generators (backend mirror; 2026-09-03 full-stack) ───

const FRAMEWORK_DEP_LEVELS = '../../../../../../../'

/**
 * The namespace workspace `Sources/{ns}/Cargo.toml` (mount-only shell).
 * Members = one crate per service; workspace deps pinned to the versions the
 * upstream namespace workspaces use. Compiles only where the Framework crates
 * resolve (AliothStudio checkout or a provisioned content root).
 */
export function generateNamespaceWorkspace(namespace: string, serviceIds: readonly string[]): string {
  const members = serviceIds.map(id => `    "Sources/Apps/Services/${id}/backend",`)
  return `# ${namespace} 开发 workspace：独立 target/ 和 Cargo.lock（dsh-alioth scaffold 生成）\n[workspace]\nresolver = "2"\nmembers = [\n${members.join('\n')}\n]\n\nexclude = ["**/vendor/**"]\n\n[workspace.package]\nversion = "0.1.0"\nedition = "2021"\nlicense = "Apache-2.0"\n\n[workspace.dependencies]\ntokio = { version = "1", features = ["full"] }\nactix-web = "4"\nsqlx = { version = "0.9.0", features = ["runtime-tokio", "postgres", "uuid", "chrono", "macros", "migrate", "rust_decimal"] }\nserde = { version = "1", features = ["derive"] }\nserde_json = "1"\nchrono = { version = "0.4", features = ["serde"] }\nthiserror = "2"\nasync-trait = "0.1"\ndotenvy = "0.15"\nlog = "0.4"\nuuid = { version = "1", features = ["v4", "serde"] }\n`
}

/**
 * Service crate shell: `backend/Cargo.toml` + `backend/src/lib.rs`. The shell
 * is mount-only (upstream spec: the lib registers the service scope; business
 * code is authored by the model in gated workflow steps, never scaffolded).
 */
export function generateServiceCrate(namespace: string, serviceId: string): Readonly<Record<string, string>> {
  const cargoToml = `[package]\nname = "alioth-service-${serviceId}"\nversion.workspace = true\nedition.workspace = true\nlicense.workspace = true\n\n[dependencies]\nactix-web = { workspace = true }\nserde = { workspace = true }\nserde_json = { workspace = true }\ncommon = { path = "${FRAMEWORK_DEP_LEVELS}/Framework/backend/common" }\ncrud = { path = "${FRAMEWORK_DEP_LEVELS}/Framework/backend/crud" }\n\n[lib]\npath = "src/lib.rs"\n`
  const libRs = `//! # ${serviceId} — ${namespace} 服务壳\n//!\n//! 壳纯挂载（gated code authoring）：业务路由/DTO 由模型在 workflow 门禁步骤\n//! 内编写并经 cargo check 验收；本壳只注册服务作用域。\n\nuse actix_web::web;\n\n/// 注册 ${serviceId} 服务的路由作用域。\npub fn register_service_routes(cfg: &mut web::ServiceConfig) {\n    cfg.service(web::scope("/service/${serviceId}"));\n}\n`
  return { 'backend/Cargo.toml': cargoToml, 'backend/src/lib.rs': libRs }
}
