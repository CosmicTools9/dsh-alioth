/**
 * 运行期验证产物（runtime-verify）契约：canonical / degraded **互斥**、门只由 `/verdict == "ready"`
 * 解除、证据脱敏、缺失不做通过。对齐上游 `verify_runtime.rs` + NS_APP_RUNTIME_HOSTING_SPEC §9.3/§9.7。
 */
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { describe, expect, it } from 'vitest'
import {
  buildRuntimeVerify,
  readRuntimeVerify,
  redactRuntimeEvidence,
  runtimeVerdictOutcome,
  runtimeVerifyUnlock,
  writeRuntimeVerify,
  RUNTIME_VERIFY_DEGRADED_NAME,
  RUNTIME_VERIFY_NAME,
  RUNTIME_VERIFY_SCHEMA,
} from '../src/runtime-verify.ts'

async function appDir(): Promise<string> {
  return mkdtemp(path.join(tmpdir(), 'runtime-verify-'))
}

describe('runtimeVerdictOutcome', () => {
  it('only ready|failed are real runs; only ready unlocks the gate', () => {
    expect(runtimeVerdictOutcome('ready')).toEqual({ canonical: true, degraded: false, unlocksGate: true })
    expect(runtimeVerdictOutcome('failed')).toEqual({ canonical: true, degraded: false, unlocksGate: false })
    expect(runtimeVerdictOutcome('started')).toEqual({ canonical: false, degraded: true, unlocksGate: false })
    expect(runtimeVerdictOutcome('environment_unreachable')).toEqual({ canonical: false, degraded: true, unlocksGate: false })
  })
})

describe('buildRuntimeVerify', () => {
  it('marks ready as passed and failed as a real run that is not passed', () => {
    const ready = buildRuntimeVerify({ namespace: 'ns', app: 'a', verdict: 'ready', container: 'c1' })
    expect(ready.schema).toBe(RUNTIME_VERIFY_SCHEMA)
    expect(ready.passed).toBe(true)
    expect(ready.degraded).toBe(false)

    const failed = buildRuntimeVerify({ namespace: 'ns', app: 'a', verdict: 'failed', failures: ['probe /health 503'] })
    expect(failed.passed).toBe(false)
    expect(failed.degraded).toBe(false)
    expect(failed.failures).toEqual(['probe /health 503'])
  })

  it('keeps degraded evidence honest: not passed, cleanup reported as not run', () => {
    const doc = buildRuntimeVerify({ namespace: 'ns', app: 'a', verdict: 'environment_unreachable' })
    expect(doc.passed).toBe(false)
    expect(doc.degraded).toBe(true)
    expect(doc.cleanup).toEqual({ ran: false, ok: null, detail: '未启动容器（环境不可达）' })
    expect(doc.container).toBe('')
  })

  it('redacts credential-shaped fields anywhere in the evidence', () => {
    const doc = buildRuntimeVerify({
      namespace: 'ns',
      app: 'a',
      verdict: 'ready',
      probes: [{ url: 'http://x/health', via: 'exec', status: 200 }],
      cleanup: { ran: true, ok: true, detail: 'down ok' },
      logs_cmd: 'logs --tail 40 k1',
    })
    // 脱敏按**键名**判定值域：命令串是平台给的普通文本（§9.3 另禁其内嵌凭据），保持原样。
    expect(doc.logs_cmd).toBe('logs --tail 40 k1')
    const nested = redactRuntimeEvidence({ a: [{ password: 'p', database_url: 'postgres://u:p@h/db' }], keep: 1 })
    expect(nested).toEqual({ a: [{ password: '[redacted]', database_url: '[redacted]' }], keep: 1 })
  })
})

describe('writeRuntimeVerify', () => {
  it('writes canonical for a real run and removes a stale degraded file', async () => {
    const dir = await appDir()
    try {
      await writeFile(path.join(dir, RUNTIME_VERIFY_DEGRADED_NAME), '{}\n', 'utf8')
      const written = await writeRuntimeVerify(dir, buildRuntimeVerify({ namespace: 'ns', app: 'a', verdict: 'ready' }))
      expect(written.canonical).toBe(path.join(dir, RUNTIME_VERIFY_NAME))
      expect(written.unlocksGate).toBe(true)
      const state = await readRuntimeVerify(dir)
      expect(state.canonical?.verdict).toBe('ready')
      expect(state.degraded).toBeNull()
    } finally {
      await rm(dir, { recursive: true, force: true })
    }
  })

  it('a failed run keeps canonical but must not unlock the gate, and a later degraded run deletes canonical', async () => {
    const dir = await appDir()
    try {
      const failed = await writeRuntimeVerify(dir, buildRuntimeVerify({ namespace: 'ns', app: 'a', verdict: 'failed' }))
      expect(failed.unlocksGate).toBe(false)
      expect(JSON.parse(await readFile(path.join(dir, RUNTIME_VERIFY_NAME), 'utf8')).verdict).toBe('failed')

      const degraded = await writeRuntimeVerify(dir, buildRuntimeVerify({ namespace: 'ns', app: 'a', verdict: 'environment_unreachable' }))
      expect(degraded.degraded).toBe(path.join(dir, RUNTIME_VERIFY_DEGRADED_NAME))
      const state = await readRuntimeVerify(dir)
      expect(state.canonical).toBeNull()
      expect(state.degraded?.verdict).toBe('environment_unreachable')
    } finally {
      await rm(dir, { recursive: true, force: true })
    }
  })

  it('an inconclusive started run writes neither file and leaves existing evidence alone', async () => {
    const dir = await appDir()
    try {
      await writeRuntimeVerify(dir, buildRuntimeVerify({ namespace: 'ns', app: 'a', verdict: 'ready' }))
      const out = await writeRuntimeVerify(dir, buildRuntimeVerify({ namespace: 'ns', app: 'a', verdict: 'started' }))
      expect(out.canonical).toBeUndefined()
      expect(out.degraded).toBeUndefined()
      expect(out.unlocksGate).toBe(false)
      expect((await readRuntimeVerify(dir)).canonical?.verdict).toBe('ready')
    } finally {
      await rm(dir, { recursive: true, force: true })
    }
  })

  it('treats missing or corrupt evidence as absent (absence is never a pass)', async () => {
    const dir = await appDir()
    try {
      expect(await readRuntimeVerify(dir)).toEqual({ canonical: null, degraded: null })
      await writeFile(path.join(dir, RUNTIME_VERIFY_NAME), 'not json\n', 'utf8')
      expect((await readRuntimeVerify(dir)).canonical).toBeNull()
    } finally {
      await rm(dir, { recursive: true, force: true })
    }
  })
})

describe('runtimeVerifyUnlock', () => {
  it('unlocks on the content predicate /verdict == ready, never on file existence', async () => {
    const dir = await appDir()
    try {
      const condition = runtimeVerifyUnlock(dir)
      expect(condition.kind).toBe('artifact-json-pointer')
      expect(condition.path).toBe(path.join(dir, RUNTIME_VERIFY_NAME))
      expect(condition.pointer).toBe('/verdict')
      expect(condition.equals).toBe('ready')

      await writeRuntimeVerify(dir, buildRuntimeVerify({ namespace: 'ns', app: 'a', verdict: 'failed' }))
      const failedDoc = JSON.parse(await readFile(condition.path, 'utf8'))
      expect(failedDoc[condition.pointer.slice(1)]).not.toBe(condition.equals)
    } finally {
      await rm(dir, { recursive: true, force: true })
    }
  })
})
