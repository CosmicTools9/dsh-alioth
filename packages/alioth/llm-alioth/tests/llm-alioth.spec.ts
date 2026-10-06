/**
 * `llm-alioth` — the BYOK-first key order is the plugin's whole behavioral
 * difference from the harness row, so it gets direct coverage here (mounted on
 * a real Context for the plugin shape, with the key seams injected). The
 * provider/adapter wiring itself is exercised by the composition smoke and
 * tree-assembly gates.
 */
import { describe, expect, it } from 'vitest'
import * as llmAlioth from '../src/index.ts'
import { resolveRequestKey } from '../src/index.ts'

describe('resolveRequestKey (BYOK → credentials → ambient)', () => {
  const base = {
    account: null,
    apiKeyFor: undefined,
    credentials: undefined,
    ambient: 'sk-platform',
    ref: 'DEEPSEEK_API_KEY',
  }

  it('prefers the account’s own key when the BYOK seam has one', async () => {
    const result = await resolveRequestKey({
      ...base,
      account: 'U-ada',
      apiKeyFor: async account => (account === 'U-ada' ? 'sk-own-ada' : null),
    })
    expect(result).toEqual({ key: 'sk-own-ada', source: 'byok' })
  })

  it('falls through to the platform credential when the account has none', async () => {
    const result = await resolveRequestKey({
      ...base,
      account: 'U-eve',
      apiKeyFor: async () => null,
    })
    expect(result).toEqual({ key: 'sk-platform', source: 'ambient' })
    // And through the credentials service when one is mounted.
    const viaStore = await resolveRequestKey({
      ...base,
      account: 'U-eve',
      apiKeyFor: async () => null,
      credentials: { resolve: async () => ({ value: 'sk-stored' }) },
    })
    expect(viaStore).toEqual({ key: 'sk-stored', source: 'credentials' })
  })

  it('uses the platform key for anonymous dispatches (headless)', async () => {
    const result = await resolveRequestKey(base)
    expect(result).toEqual({ key: 'sk-platform', source: 'ambient' })
  })

  it('fails loud when nothing usable is configured (never an empty key)', async () => {
    await expect(resolveRequestKey({ ...base, ambient: undefined })).rejects.toThrow(/MISSING_CREDENTIAL|no API key/)
    await expect(resolveRequestKey({
      ...base,
      account: 'U-eve',
      apiKeyFor: async () => '',
      ambient: undefined,
    })).rejects.toThrow(/no API key/)
  })
})

describe('plugin shape', () => {
  it('exposes the harness-compatible name/inject/config surface', () => {
    expect(llmAlioth.name).toBe('llm-alioth')
    expect(llmAlioth.inject).toEqual(['llm'])
    expect(typeof llmAlioth.apply).toBe('function')
    // Same schemastery config as the harness row it replaces.
    expect(typeof llmAlioth.Config).toBe('function')
  })
})
