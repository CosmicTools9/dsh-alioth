/**
 * The model-version dependency: what an artifact declares (`min_alioth_version` /
 * `aliothVersion`) versus what a deployment provides. These helpers are the single
 * comparison used by generation (stamping), `alioth_app_inspect` and the console panel —
 * so they are asserted directly, including the undecidable cases both sides must not guess at.
 */
import { describe, expect, it } from 'vitest'
import { compareModelVersions, DEFAULT_MIN_ALIOTH_VERSION, displayModelVersion, generateApp, modelVersionAnchor, parseModelVersion, satisfiesModelVersion, validateArtifact } from '../src/index.ts'

describe('model-version parsing and comparison', () => {
  it('parses release-shaped triples, tolerating a leading v', () => {
    expect(parseModelVersion('10.0.34')).toEqual([10, 0, 34])
    expect(parseModelVersion('v10.0.34')).toEqual([10, 0, 34])
    expect(parseModelVersion(' 10.0.0 ')).toEqual([10, 0, 0])
  })

  it('rejects anything that is not a release triple', () => {
    for (const value of ['', '10.0', '10.0.0-fixture', 'latest', 'v10.0.34-rc1', '10.0.0.1']) {
      expect(parseModelVersion(value), value).toBeNull()
    }
  })

  it('compares numerically, not lexically', () => {
    expect(compareModelVersions('10.0.9', '10.0.34')).toBe(-1)
    expect(compareModelVersions('10.0.34', '10.0.34')).toBe(0)
    expect(compareModelVersions('10.1.0', '10.0.34')).toBe(1)
    expect(compareModelVersions('11.0.0', '10.99.99')).toBe(1)
    expect(compareModelVersions('10.0.0', 'not-a-version')).toBeNull()
  })

  it('resolves a declared minimum against the available model', () => {
    expect(satisfiesModelVersion('10.0.0', '10.0.34')).toBe(true)
    expect(satisfiesModelVersion('10.0.34', '10.0.34')).toBe(true)
    expect(satisfiesModelVersion('10.1.0', '10.0.34')).toBe(false)
    // Undecidable stays null — an unparseable side is never silently treated as satisfied.
    expect(satisfiesModelVersion('10.0.0', '0.0.0-fixture')).toBeNull()
    expect(satisfiesModelVersion('', '10.0.34')).toBeNull()
  })

  it('anchors artifacts on the live model, normalised, else on the contract floor', () => {
    expect(modelVersionAnchor('v10.0.34')).toBe('10.0.34')
    expect(modelVersionAnchor('10.0.34')).toBe('10.0.34')
    // A model source that is not a release (fixtures, git refs) must not become an artifact's
    // declared dependency: the artifact falls back to the floor it can actually promise.
    expect(modelVersionAnchor('0.0.0-fixture')).toBe(DEFAULT_MIN_ALIOTH_VERSION)
    expect(modelVersionAnchor(undefined)).toBe(DEFAULT_MIN_ALIOTH_VERSION)
  })

  it('shows a release-shaped version in the same form artifacts carry, others verbatim', () => {
    expect(displayModelVersion('v10.0.34')).toBe('10.0.34')
    expect(displayModelVersion('10.0.34')).toBe('10.0.34')
    expect(displayModelVersion('0.0.0-fixture')).toBe('0.0.0-fixture')
    expect(displayModelVersion('')).toBe('')
  })
})

describe('generateApp model dependency', () => {
  /** Minimal app spec; the contract requires the rest of the required set, not this test. */
  function spec(overrides: Record<string, unknown> = {}) {
    return {
      id: '946462018160351133',
      namespace: 'Demo',
      code: 'demo-app',
      name: 'Demo 应用',
      modules: [{ id: 'inventory', name: '库存' }],
      ...overrides,
    }
  }

  it('declares the caller\'s model version as the artifact minimum', () => {
    const generated = generateApp(spec({ minAliothVersion: '10.0.34' }))
    expect(generated.app['min_alioth_version']).toBe('10.0.34')
    // The declared minimum is part of the contract, so a stamped app must still validate.
    expect(validateArtifact('app', generated.app).valid).toBe(true)
  })

  it('falls back to the contract floor when the caller omits the version', () => {
    expect(generateApp(spec()).app['min_alioth_version']).toBe(DEFAULT_MIN_ALIOTH_VERSION)
  })
})
