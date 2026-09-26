/**
 * Pure artifact generators: `app.json` + per-module `module.json` from a
 * structural spec. No I/O — callers own persistence. Generated output always
 * validates against the contracts in `contracts/`.
 * @module @dsh-alioth/gen-alioth/generate
 */

import { DEFAULT_MIN_ALIOTH_VERSION } from './version.ts'

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
  /**
   * Lifecycle status. The AppAgent evaluation requires the field
   * (`evaluate.rs:234-242` required set includes `status`), so the generator emits it
   * rather than leaving a downstream stage to patch the artifact after the fact.
   */
  readonly status?: string
  /**
   * The model version this app is generated against — stamped into
   * `min_alioth_version`, the artifact's declared dependency on the model. Callers pass the
   * deployment's live model version; omitting it declares the contract floor.
   */
  readonly minAliothVersion?: string
}

export interface GeneratedApp {
  readonly app: Record<string, unknown>
  readonly modules: readonly Record<string, unknown>[]
}

const DEFAULT_VERSION = '0.1.0'
/** Newly generated apps are under construction; `developing` is in the AppAgent evaluation's enum. */
const DEFAULT_APP_STATUS = 'developing'

/** The app-level extensions per the distribution's artifact tree (DESIGN_INTENT). */
export const EXTENSION_FILES = ['constraints', 'rules', 'statemachines', 'workflows'] as const

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

export interface BlockSpec {
  readonly id: string
  readonly namespace: string
  readonly name?: string
  readonly version?: string
  readonly aliothVersion?: string
  /** 归属模块 `{ns}/{moduleId}`；骨架期缺省不写——自指即悬空引用（R6 判失败）。 */
  readonly ownerModule?: string
  readonly sharingMode?: 'single' | 'shared'
  readonly consumers?: readonly string[]
  readonly services?: readonly string[]
}

/**
 * Build a block.json **scaffold**, mirroring upstream `create_block_scaffold`
 * (`Meta/backend/app-agent/src/tools.rs`): the skeleton carries the empty fields the
 * later stages own — `block: ""` (业务编码由精化阶段回填) and `coordinates: null`
 * (坐标由本体映射阶段回填；`check-block-json.ts` 的 R5 对 null 跳过校验)。
 *
 * One deliberate addition over upstream: `aliothVersion` is filled in here, because
 * this generator knows the target model version while upstream leaves the field to
 * `auto-fix-versions.ts`. Our own contract (and R2) requires the key, so emitting a
 * scaffold that is guaranteed to violate it would be a self-inflicted gate failure.
 */
export function generateBlock(spec: BlockSpec): Record<string, unknown> {
  const sharing: Record<string, unknown> = {
    mode: spec.sharingMode ?? 'single',
    consumers: [...(spec.consumers ?? [])],
  }
  if (spec.ownerModule !== undefined) {
    sharing['ownerModule'] = spec.ownerModule
  }
  return {
    id: spec.id,
    namespace: spec.namespace,
    name: spec.name ?? spec.id,
    version: spec.version ?? DEFAULT_VERSION,
    // 骨架期尚未构建原型：按 §4.1 的仓内约定（仅含 llm-tsx 的块均声明 v1）取 v1。
    prototypeVersion: 'v1',
    block: '',
    services: [...(spec.services ?? [])],
    sharing,
    coordinates: null,
    aliothVersion: spec.aliothVersion ?? DEFAULT_MIN_ALIOTH_VERSION,
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
    status: spec.status ?? DEFAULT_APP_STATUS,
    min_alioth_version: spec.minAliothVersion ?? DEFAULT_MIN_ALIOTH_VERSION,
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

/** The service.json artifact (contract: `service`; layout-faithful to the
 *  upstream mirror output — all 15 keys). */
export interface ServiceSpec {
  readonly id: string
  readonly namespace: string
  readonly domain: string
  readonly services: readonly string[]
  readonly layer: number
  readonly dtoDependencies: readonly string[]
  /** DTO surface; defaults to the generic refs/queries contract. */
  readonly dtoExposes?: { readonly refs?: readonly string[]; readonly queries?: readonly string[] }
  readonly backendCrate: string
  readonly hasBackend: boolean
  readonly hasFrontend: boolean
  readonly version?: string
  /** Alioth model version the service targets; defaults to the minimum. */
  readonly aliothVersion?: string
  readonly publishes?: readonly string[]
  readonly subscribes?: readonly string[]
  readonly ontology: { readonly entities: readonly ServiceEntitySpec[] }
}

/** Build a service.json from an ontology spec; always passes the service contract. */
export function generateService(spec: ServiceSpec): Record<string, unknown> {
  return {
    id: spec.id,
    namespace: spec.namespace,
    domain: spec.domain,
    services: [...spec.services],
    layer: spec.layer,
    dtoDependencies: [...spec.dtoDependencies],
    dtoExposes: {
      refs: [...(spec.dtoExposes?.refs ?? [])],
      queries: [...(spec.dtoExposes?.queries ?? ['list_refs', 'get_refs'])],
    },
    backendCrate: spec.backendCrate,
    hasBackend: spec.hasBackend,
    hasFrontend: spec.hasFrontend,
    version: spec.version ?? DEFAULT_VERSION,
    aliothVersion: spec.aliothVersion ?? DEFAULT_MIN_ALIOTH_VERSION,
    publishes: [...(spec.publishes ?? [])],
    subscribes: [...(spec.subscribes ?? [])],
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
  const cargoToml = `[package]\nname = "${namespace.toLowerCase()}-service-${serviceId}"\nversion.workspace = true\nedition.workspace = true\nlicense.workspace = true\n\n[dependencies]\nactix-web = { workspace = true }\nserde = { workspace = true }\nserde_json = { workspace = true }\ncommon = { path = "${FRAMEWORK_DEP_LEVELS}/Framework/backend/common" }\ncrud = { path = "${FRAMEWORK_DEP_LEVELS}/Framework/backend/crud" }\n\n[lib]\npath = "src/lib.rs"\n`
  const libRs = `//! # ${serviceId} — ${namespace} 服务壳\n//!\n//! 壳纯挂载（gated code authoring）：业务路由/DTO 由模型在 workflow 门禁步骤\n//! 内编写并经 cargo check 验收；本壳只注册服务作用域。\n\nuse actix_web::web;\n\n/// 注册 ${serviceId} 服务的路由作用域。\npub fn register_service_routes(cfg: &mut web::ServiceConfig) {\n    cfg.service(web::scope("/service/${serviceId}"));\n}\n`
  return { 'backend/Cargo.toml': cargoToml, 'backend/src/lib.rs': libRs }
}
