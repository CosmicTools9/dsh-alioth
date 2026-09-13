import { fileURLToPath } from 'node:url'
import path from 'node:path'
import { describe, expect, it } from 'vitest'
import { unreachableGatePrograms } from '@dsh-alioth/skill-alioth'

const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const VENDOR = path.join(REPO_ROOT, 'packages', 'alioth', 'env-alioth', 'vendor')

describe('vendored adapter gate programs', () => {
  it('resolves every gate script the vendored adapters invoke', () => {
    // A missing script spawns ENOENT, classifies as path-missing (not
    // LLM-fixable) and stalls the track permanently — the sync set must carry
    // each script and its data. Regression: check-block-json.ts and
    // audit-service-spec.ts were referenced by alioth-block 1.1 / alioth-service
    // 1.5 while absent from the vendor tree.
    expect(unreachableGatePrograms(path.join(VENDOR, 'skill-adapters'), VENDOR)).toEqual([])
  })
})
