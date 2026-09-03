import { describe, expect, it } from 'vitest'
import { EXTENSION_FILES, generateApp, generateExtensions, generateModule, generateService, generateNamespaceWorkspace, generateServiceCrate, sourceModuleDirs, sourceServiceDirs, validateArtifact, validateArtifactWith, type ArtifactSchemas } from '../src/index.ts'

/** Self-contained valid app artifact (hand-written test data). */
const VALID_APP = {
  id: '946462018160351133',
  code: 'demo-app',
  namespace: 'Demo',
  name: 'Demo 应用',
  version: '0.1.0',
  config: {
    modules: ['inventory', 'demand'],
    blocks: ['block-list-inventory'],
  },
  permissions: {
    defaultRoles: ['admin', 'user'],
    adminRoles: ['admin'],
  },
  routing: { base: '/apps/demo-app', defaultRoute: '/inventory' },
  navigation: [{ group: '系统管理', icon: 'Settings', modules: ['inventory', 'demand'] }],
  min_alioth_version: '10.0.0',
}

/** Valid module.json (hand-written test data, module-contract shape). */
const VALID_MODULE = {
  id: 'inventory',
  namespace: 'Demo',
  name: '库存',
  category: 'business',
  status: 'planned',
  routePrefix: '/inventory',
  icon: 'AppstoreOutlined',
  hasBackend: false,
  hasFrontend: true,
  version: '0.1.0',
  selectable: true,
  description: '库存管理',
}

/** Valid service.json (hand-written test data, service-contract shape). */
const VALID_SERVICE = {
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
  it('accepts a valid app artifact', () => {
    expect(validateArtifact('app', VALID_APP)).toEqual({ valid: true, errors: [] })
  })

  it('rejects missing required fields with paths', () => {
    const { min_alioth_version: _omit, ...partial } = VALID_APP
    const result = validateArtifact('app', partial)
    expect(result.valid).toBe(false)
    expect(result.errors.some(error => error.includes('min_alioth_version'))).toBe(true)
  })

  it('rejects namespaces that violate the Gateway pattern', () => {
    const result = validateArtifact('app', { ...VALID_APP, namespace: 'alioth' })
    expect(result.valid).toBe(false)
    expect(result.errors.some(error => error.includes('namespace'))).toBe(true)
  })

  it('rejects unknown top-level keys (drift guard)', () => {
    expect(validateArtifact('app', { ...VALID_APP, surprise: true }).valid).toBe(false)
  })
})

describe('gen-alioth module contract', () => {
  it('accepts a valid module.json', () => {
    expect(validateArtifact('module', VALID_MODULE)).toEqual({ valid: true, errors: [] })
  })

  it('rejects a module without version', () => {
    const result = validateArtifact('module', { id: 'm', namespace: 'N', name: 'M' })
    expect(result.valid).toBe(false)
  })
})

describe('gen-alioth service contract', () => {
  it('accepts a valid service.json', () => {
    expect(validateArtifact('service', VALID_SERVICE)).toEqual({ valid: true, errors: [] })
  })

  it('rejects an entity without table', () => {
    const broken = {
      ...VALID_SERVICE,
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
  it('regenerates a valid app shape from its spec', () => {
    const { app, modules } = generateApp({
      id: 'new-id',
      namespace: VALID_APP.namespace,
      code: VALID_APP.code,
      name: VALID_APP.name,
      version: VALID_APP.version,
      modules: [{ id: 'inventory', name: '库存' }, { id: 'demand', name: '需求' }],
      blocks: [...VALID_APP.config.blocks],
      navigation: VALID_APP.navigation,
      defaultRoles: VALID_APP.permissions.defaultRoles,
      adminRoles: VALID_APP.permissions.adminRoles,
    })
    expect({ ...app, id: VALID_APP.id }).toEqual(VALID_APP)
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

  it('passes description through when set and omits it when not', () => {
    const withDescription = generateApp({
      id: '1', namespace: 'Demo', code: 'demo', name: 'Demo', modules: [],
      description: 'one-liner',
    })
    expect(withDescription.app.description).toBe('one-liner')
    expect(validateArtifact('app', withDescription.app).valid).toBe(true)
    const without = generateApp({ id: '1', namespace: 'Demo', code: 'demo', name: 'Demo', modules: [] })
    expect('description' in without.app).toBe(false)
  })
})

describe('gen-alioth generateModule', () => {
  it('builds a contract-valid module.json following the owning app version', () => {
    const module = generateModule({ namespace: 'Demo', version: '2.3.0' }, { id: 'alpha', name: 'A', description: 'd', icon: 'Cpu' })
    expect(validateArtifact('module', module)).toEqual({ valid: true, errors: [] })
    expect(module).toMatchObject({
      id: 'alpha',
      namespace: 'Demo',
      name: 'A',
      category: 'business',
      status: 'planned',
      version: '2.3.0',
      routePrefix: '/alpha',
      icon: 'Cpu',
      description: 'd',
    })
  })

  it('matches the module shape generateApp produces for the same spec', () => {
    const owner = { namespace: 'Demo', version: '0.1.0' }
    const spec = { id: 'alpha', name: 'A', description: 'd' }
    const { modules } = generateApp({ id: '1', ...owner, code: 'demo', name: 'Demo', modules: [spec] })
    expect(generateModule(owner, spec)).toEqual(modules[0])
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
    // Gateway ExtensionLoader contract: the four standard files must be
    // top-level YAML sequences (`Vec<T>`), not map wrappers (2026-08-23 fix).
    expect(extensions['constraints.yaml']!.trimEnd()).toMatch(/\[\]\s*$/)
  })

  it('lists one source module dir per module', () => {
    const spec = { id: '1', namespace: 'Demo', code: 'demo', name: 'Demo', modules: [{ id: 'alpha', name: 'A' }] }
    expect(sourceModuleDirs(spec.modules)).toEqual(['Sources/Apps/Modules/alpha'])
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

  it('lists one service source dir per service id (mirror layout)', () => {
    expect(sourceServiceDirs([{ id: 'demo-inventory-service' }])).toEqual(['Sources/Apps/Services/demo-inventory-service'])
  })
})

describe('gen-alioth sources scaffold generators', () => {
  it('generates a namespace workspace manifest with one member per service', () => {
    const manifest = generateNamespaceWorkspace('Demo', ['inventory', 'sales-order'])
    expect(manifest).toContain('members = [')
    expect(manifest).toContain('"Sources/Apps/Services/inventory/backend"')
    expect(manifest).toContain('"Sources/Apps/Services/sales-order/backend"')
    expect(manifest).toContain('edition = "2021"')
  })

  it('generates a mount-only service crate shell', () => {
    const files = generateServiceCrate('Demo', 'inventory')
    expect(Object.keys(files).sort()).toEqual(['backend/Cargo.toml', 'backend/src/lib.rs'])
    expect(files['backend/Cargo.toml']).toContain('name = "alioth-service-inventory"')
    expect(files['backend/Cargo.toml']).toContain('../../../../../../..//Framework/backend/common')
    expect(files['backend/src/lib.rs']).toContain('pub fn register_service_routes')
    expect(files['backend/src/lib.rs']).toContain('/service/inventory')
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
