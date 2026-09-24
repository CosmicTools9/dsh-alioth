/**
 * auth-web-alioth — the paid source package.
 *
 * This allowlist is deliberately NOT the prototype one: what the console shows
 * everyone and what a subscription delivers must stay separate rules. The tests
 * pin both the shape rule and the walk (pruning, symlink refusal, cap).
 */
import { describe, expect, it, beforeEach, afterEach } from 'vitest'
import { mkdir, mkdtemp, rm, symlink, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { collectSourcePackage, isSourcePackagePath, SourcePackageTooLargeError } from '../src/source-package.ts'

let root: string
let appDir: string

beforeEach(async () => {
  root = await mkdtemp(path.join(tmpdir(), 'dsh-alioth-srcpkg-'))
  appDir = path.join(root, 'Pre-Proc', 'U-ada', 'Apps', 'default')
  await mkdir(appDir, { recursive: true })
})

afterEach(async () => {
  await rm(root, { recursive: true, force: true })
})

async function write(relative: string, contents = 'x'): Promise<void> {
  const file = path.join(appDir, relative)
  await mkdir(path.dirname(file), { recursive: true })
  await writeFile(file, contents, 'utf8')
}

describe('isSourcePackagePath', () => {
  it('delivers contract files and the three source trees', () => {
    for (const allowed of [
      'app.json',
      'prototype.html',
      'Sources/main.rs',
      'Sources/Modules/stock/module.json',
      'modules/stock/module.json',
      'extensions/rules.yaml',
    ]) {
      expect(isSourcePackagePath(allowed), allowed).toBe(true)
    }
  })

  it('excludes everything else, including the visible/other surfaces', () => {
    for (const denied of [
      '',
      'Deploy/default/app.json',
      'notes.md',
      'AppAgentTraces/closure-audit/1.json',
      'e2e-report.json',
      'extension-verify.json',
      'tools/x',
      'Sources',
      'Sources/../secrets.txt',
      'Sources\\main.rs',
      '../app.json',
      'Sources/main.rs\0.txt',
    ]) {
      expect(isSourcePackagePath(denied), denied).toBe(false)
    }
  })
})

describe('collectSourcePackage', () => {
  it('collects the three trees plus the contract files, pruning build output', async () => {
    await write('app.json', '{"code":"default"}')
    await write('prototype.html', '<html></html>')
    await write('Sources/main.rs', 'fn main() {}')
    await write('Sources/target/debug/blob', 'BUILD OUTPUT')
    await write('Sources/node_modules/pkg/index.js', 'DEP')
    await write('modules/stock/module.json', '{}')
    await write('extensions/rules.yaml', 'form: rules')
    await write('AppAgentTraces/closure-audit/1.json', '{}')

    const pkg = await collectSourcePackage(appDir, 'default')

    // Archive order is the module's contract: root files, then modules,
    // extensions, Sources (each lexical inside itself).
    expect(pkg.entries.map(entry => entry.name)).toEqual([
      'default/app.json',
      'default/prototype.html',
      'default/modules/stock/module.json',
      'default/extensions/rules.yaml',
      'default/Sources/main.rs',
    ])
    expect(pkg.bytes).toBeGreaterThan(0)
    // Deterministic: the same app always yields the same archive order.
    expect((await collectSourcePackage(appDir, 'default')).entries.map(entry => entry.name))
      .toEqual(pkg.entries.map(entry => entry.name))
    expect(new TextDecoder().decode(pkg.entries[4]?.data)).toBe('fn main() {}')
    expect(pkg.entries[4]?.mtime).toBeInstanceOf(Date)
  })

  it('refuses to smuggle the deployment in through a symlink', async () => {
    await write('app.json', '{}')
    await mkdir(path.join(root, 'outside'), { recursive: true })
    await writeFile(path.join(root, 'outside', 'secret.txt'), 'SECRET')
    await mkdir(path.join(appDir, 'Sources'), { recursive: true })
    await symlink(path.join(root, 'outside'), path.join(appDir, 'Sources', 'linked'))

    const pkg = await collectSourcePackage(appDir, 'default')

    expect(pkg.entries.map(entry => entry.name)).toEqual(['default/app.json'])
  })

  it('fails loud over the size cap instead of truncating', async () => {
    await write('app.json', '{}')
    await write('Sources/big.rs', 'x'.repeat(2048))

    await expect(collectSourcePackage(appDir, 'default', 512)).rejects.toBeInstanceOf(SourcePackageTooLargeError)
    // Under the cap it collects normally.
    await expect(collectSourcePackage(appDir, 'default', 4096)).resolves.toMatchObject({ bytes: expect.any(Number) })
  })

  it('returns an empty package for an app with nothing on disk', async () => {
    expect((await collectSourcePackage(appDir, 'default')).entries).toEqual([])
  })
})
