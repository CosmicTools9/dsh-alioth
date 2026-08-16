import { describe, expect, it } from 'vitest'
import { EXTENSION_FILES, generateApp, generateExtensions, generateService, sourceModuleDirs, sourceServiceDirs, validateArtifact, validateArtifactWith, type ArtifactSchemas } from '../src/index.ts'

/** Golden mirror: `Pre-Proc/Alioth/Apps/ai-i-need-a/app.json` (same shape as the tool-alioth fixture). */
const GOLDEN_APP = {
  id: '946462018160351133',
  code: 'ai-i-need-a',
  namespace: 'Alioth',
  name: 'ai-i-need-a',
  version: '0.1.0',
  config: {
    modules: ['inventory', 'demand'],
    blocks: ['block-list-inventory'],
  },
  permissions: {
    defaultRoles: ['admin', 'user'],
    adminRoles: ['admin'],
  },
  routing: { base: '/apps/ai-i-need-a', defaultRoute: '/inventory' },
  navigation: [{ group: '系统管理', icon: 'Settings', modules: ['inventory', 'demand'] }],
  min_alioth_version: '10.0.0',
}

/** Real `module.json` of the AppCreator service (from the model distribution). */
const SERVICE_MODULE = {
  id: 'app-creator',
  namespace: 'AppCreator',
  name: 'AppCreator Service',
  category: 'service',
  status: 'active',
  routePrefix: '/app-creator',
  icon: 'Cpu',
  hasBackend: true,
  hasFrontend: true,
  version: '0.1.0',
  dependencies: ['meta'],
  selectable: true,
  techStack: ['Rust', 'PostgreSQL'],
  versions: [{ version: '0.1.0', active: true, releasedAt: '2026-07-19' }],
  description: 'AppCreator 独立服务 — 对话创建企业应用',
  prototypeVersion: 'v1',
}

/** Trimmed real `service.json` fixture from the vendored ontology-mapping crate. */
const SERVICE_ARTIFACT = {
  id: 'test-service',
  domain: '测试',
  services: ['FA'],
  layer: 1,
  dtoDependencies: [],
  backendCrate: 'alioth-service-test',
  hasBackend: true,
  hasFrontend: false,
  version: '0.1.0',
  ontology: {
    entities: [{
      name: 'TestProduct',
      table: 'isahl.zc_id_lifecycle_test_product',
      inherits: 'zc_id_lifecycle',
      coordinates: { scene: 'FE', factor: 'GBA', function: '↑_BA' },
      field_mappings: [
        { json_path: 'name', column: 'notice' },
        { json_path: 'price', column: 'qk_price', scalar: 'zc_id_scal-price' },
      ],
      relationships: [{ target: 'Category', type: 'belongsToMany', via: 'zc_id_lifecycle_r_test_product_category' }],
    }],
  },
}

describe('gen-alioth app contract', () => {
  it('accepts the golden app mirror', () => {
    expect(validateArtifact('app', GOLDEN_APP)).toEqual({ valid: true, errors: [] })
  })

  it('rejects missing required fields with paths', () => {
    const { min_alioth_version: _omit, ...partial } = GOLDEN_APP
    const result = validateArtifact('app', partial)
    expect(result.valid).toBe(false)
    expect(result.errors.some(error => error.includes('min_alioth_version'))).toBe(true)
  })

  it('rejects namespaces that violate the Gateway pattern', () => {
    const result = validateArtifact('app', { ...GOLDEN_APP, namespace: 'alioth' })
    expect(result.valid).toBe(false)
    expect(result.errors.some(error => error.includes('namespace'))).toBe(true)
  })

  it('rejects unknown top-level keys (drift guard)', () => {
    expect(validateArtifact('app', { ...GOLDEN_APP, surprise: true }).valid).toBe(false)
  })
})

describe('gen-alioth module contract', () => {
  it('accepts the real distribution module.json', () => {
    expect(validateArtifact('module', SERVICE_MODULE)).toEqual({ valid: true, errors: [] })
  })

  it('rejects a module without version', () => {
    const result = validateArtifact('module', { id: 'm', namespace: 'N', name: 'M' })
    expect(result.valid).toBe(false)
  })
})

describe('gen-alioth service contract', () => {
  it('accepts the real vendored service.json fixture', () => {
    expect(validateArtifact('service', SERVICE_ARTIFACT)).toEqual({ valid: true, errors: [] })
  })

  it('rejects an entity without table', () => {
    const broken = {
      ...SERVICE_ARTIFACT,
      ontology: { entities: [{ name: 'X', inherits: 'zc_id_lifecycle' }] },
    }
    expect(validateArtifact('service', broken).valid).toBe(false)
  })
})

describe('gen-alioth block contract', () => {
  it('accepts a minimal block and rejects a nameless one', () => {
    expect(validateArtifact('block', { id: 'block-list-inventory', name: '库存列表' })).toEqual({ valid: true, errors: [] })
    expect(validateArtifact('block', { id: 'x' }).valid).toBe(false)
  })
})

describe('gen-alioth generateApp', () => {
  it('regenerates the golden app shape from its spec', () => {
    const { app, modules } = generateApp({
      id: 'new-id',
      namespace: GOLDEN_APP.namespace,
      code: GOLDEN_APP.code,
      name: GOLDEN_APP.name,
      version: GOLDEN_APP.version,
      modules: [{ id: 'inventory', name: '库存' }, { id: 'demand', name: '需求' }],
      blocks: [...GOLDEN_APP.config.blocks],
      navigation: GOLDEN_APP.navigation,
      defaultRoles: GOLDEN_APP.permissions.defaultRoles,
      adminRoles: GOLDEN_APP.permissions.adminRoles,
    })
    expect({ ...app, id: GOLDEN_APP.id }).toEqual(GOLDEN_APP)
    expect(validateArtifact('app', app).valid).toBe(true)
    for (const module of modules) {
      expect(validateArtifact('module', module).valid).toBe(true)
    }
    expect(modules).toHaveLength(2)
  })

  it('derives routing defaults from code and first module', () => {
    const { app } = generateApp({ id: '1', namespace: 'Demo', code: 'demo-app', name: 'Demo App', modules: [{ id: 'alpha', name: 'A' }] })
    expect(app.routing).toEqual({ base: '/apps/demo-app', defaultRoute: '/alpha' })
    expect(app.navigation).toEqual([{ group: '系统管理', icon: 'Settings', modules: ['alpha'] }])
  })

  it('defaults an empty module list to root route', async () => {
    const { app } = generateApp({ id: '1', namespace: 'Demo', code: 'empty', name: 'Empty', modules: [] })
    expect(app.routing).toEqual({ base: '/apps/empty', defaultRoute: '/' })
  })
})

describe('gen-alioth app tree skeletons', () => {
  it('generates the four extension files with provenance', () => {
    const extensions = generateExtensions('demo-app')
    expect(Object.keys(extensions).sort()).toEqual([...EXTENSION_FILES].map(kind => `${kind}.yaml`))
    for (const content of Object.values(extensions)) {
      expect(content).toContain('demo-app')
      expect(content).toContain('dsh-alioth')
    }
    expect(extensions['constraints.yaml']).toContain('constraints')
  })

  it('lists one source module dir per module', () => {
    const spec = { id: '1', namespace: 'Demo', code: 'demo', name: 'Demo', modules: [{ id: 'alpha', name: 'A' }] }
    expect(sourceModuleDirs(spec.modules)).toEqual(['Sources/Modules/alpha'])
    expect(sourceModuleDirs([])).toEqual([])
  })
})

describe('gen-alioth generateService', () => {
  it('generates a service.json that passes the service contract', () => {
    const service = generateService({
      id: 'demo-inventory-service',
      domain: '库存',
      services: ['FA'],
      layer: 1,
      dtoDependencies: [],
      backendCrate: 'alioth-service-demo-inventory',
      hasBackend: true,
      hasFrontend: false,
      ontology: {
        entities: [{
          name: 'BillCheck',
          table: 'isahl.zc_id_deta-bill-check',
          inherits: 'zc_id_lifecycle',
          coordinates: { scene: 'CA', factor: 'GBA', function: '↑_AA' },
          fieldMappings: [
            { jsonPath: 'name', column: 'notice' },
            { jsonPath: 'biller', column: 'fk_biller' },
          ],
          relationships: [{ target: 'Category', type: 'belongsToMany', via: 'zc_id_lifecycle_r_test' }],
        }],
      },
    })
    expect(validateArtifact('service', service).valid).toBe(true)
    const entity = (service.ontology as { entities: Array<Record<string, unknown>> }).entities[0]
    expect(entity).toMatchObject({
      name: 'BillCheck',
      table: 'isahl.zc_id_deta-bill-check',
      coordinates: { scene: 'CA', factor: 'GBA', function: '↑_AA' },
    })
  })

  it('lists one service source dir per service id', () => {
    expect(sourceServiceDirs([{ id: 'demo-inventory-service' }])).toEqual(['Sources/Services/demo-inventory-service'])
  })
})

describe('gen-alioth injectable schemas', () => {
  it('validates against a caller-provided schema set', () => {
    const custom: ArtifactSchemas = {
      app: { type: 'object', required: ['code'], properties: { code: { type: 'string' } } },
      module: { type: 'object', required: ['id'] },
      block: { type: 'object', required: ['id'] },
      service: { type: 'object', required: ['id'] },
    }
    expect(validateArtifactWith(custom, 'app', { code: 'x' }).valid).toBe(true)
    const strict = validateArtifactWith(custom, 'app', {})
    expect(strict.valid).toBe(false)
    expect(strict.errors.some(error => error.includes('code'))).toBe(true)
  })
})
