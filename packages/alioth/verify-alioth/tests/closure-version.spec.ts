/**
 * 闭环裁决（指纹 / append-only / 升级）、产物版本回退的**全有或全无**、
 * 两段式补丁的拒绝语义。〔契约 §4/T3 必需测试〕
 * @module @dsh-alioth/verify-alioth/tests/closure-version
 */

import { afterEach, describe, expect, it } from 'vitest'
import { mkdir, readdir, readFile, rm, writeFile } from 'node:fs/promises'
import path from 'node:path'
import {
  appendClosureVerdict,
  artifactFingerprint,
  ESCALATE_THRESHOLD,
  latestMatchingVerdict,
  readClosureVerdicts,
} from '../src/closure-audit.ts'
import { KEEP_VERSIONS, listSnapshots, restoreSnapshot, snapshotArtifacts } from '../src/artifact-version.ts'
import { applyPatchProposal, proposePatch } from '../src/patch-assets.ts'
import { createTempApp, VALID_APP_JSON, type TempApp } from './fixture.ts'

const opened: TempApp[] = []
async function app(files: Readonly<Record<string, string>> = {}): Promise<TempApp> {
  const created = await createTempApp(files)
  opened.push(created)
  return created
}
afterEach(async () => {
  await Promise.all(opened.splice(0).map(entry => entry.cleanup()))
})

describe('artifactFingerprint', () => {
  it('只吃内容字节：路径/枚举顺序无关；app.json 缺失即不可得（throw）', async () => {
    const first = await app({ 'app.json': VALID_APP_JSON, 'extensions/a.yaml': '- x: 1\n', 'extensions/b.yaml': '- y: 2\n' })
    const second = await app({ 'app.json': VALID_APP_JSON, 'extensions/b.yaml': '- y: 2\n', 'extensions/a.yaml': '- x: 1\n' })
    expect(await artifactFingerprint(first.appDir)).toBe(await artifactFingerprint(second.appDir))

    await first.write('extensions/b.yaml', '- y: 3\n')
    expect(await artifactFingerprint(first.appDir)).not.toBe(await artifactFingerprint(second.appDir))

    const empty = await app()
    await expect(artifactFingerprint(empty.appDir)).rejects.toThrow(/指纹不可得/)
  })
})

describe('appendClosureVerdict — append-only + 升级', () => {
  it('seq 从 1 单调递增，记录可按 seq 升序读回，指纹匹配查询命中最新一条', async () => {
    const target = await app({ 'app.json': VALID_APP_JSON })
    const fingerprint = await artifactFingerprint(target.appDir)

    const first = await appendClosureVerdict(target.appDir, {
      app: 'tm-app',
      namespace: 'TestNS',
      verdict: 'approved',
      fingerprint,
      findings: [],
      evidence: ['eval-report.json'],
    })
    expect(first.seq).toBe(1)
    expect(await readFile(path.join(target.appDir, 'AppAgentTraces', 'closure-audit', '1.json'), 'utf8')).toContain('"seq": 1')

    const second = await appendClosureVerdict(target.appDir, {
      app: 'tm-app',
      namespace: 'TestNS',
      verdict: 'approved',
      fingerprint: 'sha256:deadbeef',
      findings: [],
      evidence: [],
    })
    expect(second.seq).toBe(2)
    expect((await readClosureVerdicts(target.appDir)).map(v => v.seq)).toEqual([1, 2])
    expect((await latestMatchingVerdict(target.appDir, fingerprint))?.seq).toBe(1)
    expect(await latestMatchingVerdict(target.appDir, 'sha256:none')).toBeNull()
  })

  it(`连续 ${ESCALATE_THRESHOLD} 次 rejected → 新裁决 verdict='escalate'（转人工）`, async () => {
    const target = await app({ 'app.json': VALID_APP_JSON })
    const record = (seq: number) => ({
      app: 'tm-app',
      namespace: 'TestNS',
      verdict: 'rejected' as const,
      fingerprint: `sha256:fp${seq}`,
      findings: [{ dimension: 'schema_validity', verdict: 'fail' as const, detail: 'app.json 违例' }],
      evidence: [],
    })

    expect((await appendClosureVerdict(target.appDir, record(1))).verdict).toBe('rejected')
    expect((await appendClosureVerdict(target.appDir, record(2))).verdict).toBe('rejected')
    const third = await appendClosureVerdict(target.appDir, record(3))
    expect(third.verdict).toBe('escalate')
    expect(third.seq).toBe(ESCALATE_THRESHOLD)
  })

  it('approved 打断 rejected 连胜（不误升级）', async () => {
    const target = await app({ 'app.json': VALID_APP_JSON })
    const base = { app: 'tm-app', namespace: 'TestNS', fingerprint: 'sha256:fp', findings: [], evidence: [] }
    await appendClosureVerdict(target.appDir, { ...base, verdict: 'rejected' })
    await appendClosureVerdict(target.appDir, { ...base, verdict: 'approved' })
    await appendClosureVerdict(target.appDir, { ...base, verdict: 'rejected' })
    expect((await readClosureVerdicts(target.appDir)).at(-1)?.verdict).toBe('rejected')
  })
})

describe('snapshotArtifacts / restoreSnapshot', () => {
  it('快照落 versions/{seq:04}-{hash8}/ 并含 MANIFEST.json；保留最近 KEEP_VERSIONS 版', async () => {
    const target = await app({ 'app.json': VALID_APP_JSON, 'extensions/a.yaml': '- x: 1\n' })
    const snap = await snapshotArtifacts(target.appDir)

    expect(snap.seq).toBe(1)
    expect(path.basename(snap.dir)).toMatch(/^0001-[0-9a-f]{8}$/)
    expect(snap.entries.map(entry => entry.rel).sort()).toEqual(['app.json', 'extensions/a.yaml'])
    expect(await readFile(path.join(snap.dir, 'MANIFEST.json'), 'utf8')).toContain('"files"')

    for (let i = 2; i <= KEEP_VERSIONS + 1; i += 1) {
      await target.write('app.json', `${JSON.stringify({ ...JSON.parse(VALID_APP_JSON) as object, version: `0.1.${i}` }, null, 2)}\n`)
      await snapshotArtifacts(target.appDir)
    }
    const seqs = await listSnapshots(target.appDir)
    expect(seqs).toHaveLength(KEEP_VERSIONS)
    expect(seqs[0]).toBe(2)
  })

  it('快照范围排除 plans/ 与 versions/', async () => {
    const target = await app({
      'app.json': VALID_APP_JSON,
      'plans/fix.md': '# plan\n',
      'notes.md': '# notes\n',
    })
    const snap = await snapshotArtifacts(target.appDir)
    expect(snap.entries.map(entry => entry.rel).sort()).toEqual(['app.json', 'notes.md'])
  })

  it('回退成功：全部文件按快照恢复', async () => {
    const target = await app({ 'app.json': VALID_APP_JSON, 'extensions/a.yaml': '- x: 1\n' })
    await snapshotArtifacts(target.appDir)
    await target.write('app.json', '{"broken":true}\n')
    await target.write('extensions/a.yaml', '- x: 999\n')

    const restored = await restoreSnapshot(target.appDir, 1)
    expect([...restored.restored].sort()).toEqual(['app.json', 'extensions/a.yaml'])
    expect(await readFile(path.join(target.appDir, 'app.json'), 'utf8')).toBe(VALID_APP_JSON)
    expect(await readFile(path.join(target.appDir, 'extensions/a.yaml'), 'utf8')).toBe('- x: 1\n')
  })

  it('任一文件指纹失配 → 拒绝整次回退，且不得半恢复任何文件', async () => {
    const target = await app({ 'app.json': VALID_APP_JSON, 'extensions/a.yaml': '- x: 1\n' })
    const snap = await snapshotArtifacts(target.appDir)
    // 破坏快照内的一个文件（模拟备份损坏）
    await writeFile(path.join(snap.dir, 'extensions', 'a.yaml'), '- x: tampered\n', 'utf8')
    // 目标侧已有「脏」内容：若发生半恢复，app.json 会被写回，可被检出
    await target.write('app.json', '{"dirty":true}\n')

    await expect(restoreSnapshot(target.appDir, 1)).rejects.toThrow(/拒绝整次回退/)
    expect(await readFile(path.join(target.appDir, 'app.json'), 'utf8')).toBe('{"dirty":true}\n')
  })

  it('快照缺文件同样拒绝整次回退；序号不存在即报错（不猜目标）', async () => {
    const target = await app({ 'app.json': VALID_APP_JSON, 'extensions/a.yaml': '- x: 1\n' })
    const snap = await snapshotArtifacts(target.appDir)
    await rm(path.join(snap.dir, 'extensions', 'a.yaml'))
    await expect(restoreSnapshot(target.appDir, 1)).rejects.toThrow(/快照缺文件/)
    await expect(restoreSnapshot(target.appDir, 42)).rejects.toThrow(/无序号 42/)
  })

  it('无可快照内容 → throw（不落空快照冒充版本）', async () => {
    const target = await app()
    await mkdir(path.join(target.appDir, 'plans'), { recursive: true })
    await target.write('plans/only.md', '# plan\n')
    await expect(snapshotArtifacts(target.appDir)).rejects.toThrow(/产物枚举为空/)
  })
})

describe('proposePatch / applyPatchProposal — 两段式拒绝语义', () => {
  const before = 'line-1\nline-2\nline-3\n'

  it('提案段不触盘：返回 diff 与基准指纹，文件保持原样', async () => {
    const target = await app({ 'app.json': VALID_APP_JSON })
    const file = await target.write('target.txt', before)
    const proposal = proposePatch({ target: file, before, after: 'line-1\nline-2-changed\nline-3\n' })

    expect(proposal.unifiedDiff).toContain('-line-2')
    expect(proposal.unifiedDiff).toContain('+line-2-changed')
    expect(proposal.baseFingerprint).toMatch(/^sha256:[0-9a-f]{64}$/)
    expect(await readFile(file, 'utf8')).toBe(before)
  })

  it('confirmed !== true → 不写盘且 reason 明示', async () => {
    const target = await app({ 'app.json': VALID_APP_JSON })
    const file = await target.write('target.txt', before)
    const proposal = proposePatch({ target: file, before, after: 'line-1\nline-2-changed\nline-3\n' })

    const result = await applyPatchProposal({ target: file, proposal, confirmed: false })
    expect(result.applied).toBe(false)
    expect(result.reason).toContain('confirmed !== true')
    expect(await readFile(file, 'utf8')).toBe(before)
  })

  it('基准指纹不符（并发改动）→ 拒绝应用且不改写', async () => {
    const target = await app({ 'app.json': VALID_APP_JSON })
    const file = await target.write('target.txt', before)
    const proposal = proposePatch({ target: file, before, after: 'line-1\nline-2-changed\nline-3\n' })
    await writeFile(file, 'concurrent-edit\n', 'utf8')

    const result = await applyPatchProposal({ target: file, proposal, confirmed: true })
    expect(result.applied).toBe(false)
    expect(result.reason).toContain('目标已变更')
    expect(await readFile(file, 'utf8')).toBe('concurrent-edit\n')
  })

  it('confirmed=true 且基准未变 → 原子写回，内容等于提案的 after', async () => {
    const target = await app({ 'app.json': VALID_APP_JSON })
    const file = await target.write('target.txt', before)
    const after = 'line-1\nline-2-changed\nline-3\n'
    const proposal = proposePatch({ target: file, before, after })

    const result = await applyPatchProposal({ target: file, proposal, confirmed: true })
    expect(result.applied).toBe(true)
    expect(await readFile(file, 'utf8')).toBe(after)
    const leftovers = (await readdir(target.appDir)).filter(name => name.includes('.tmp-patch-'))
    expect(leftovers).toEqual([])
  })

  it('跨文件套用提案 → 拒绝', async () => {
    const target = await app({ 'app.json': VALID_APP_JSON })
    const file = await target.write('target.txt', before)
    const other = await target.write('other.txt', before)
    const proposal = proposePatch({ target: file, before, after: 'line-1\nx\nline-3\n' })

    const result = await applyPatchProposal({ target: other, proposal, confirmed: true })
    expect(result.applied).toBe(false)
    expect(result.reason).toContain('不一致')
  })
})
