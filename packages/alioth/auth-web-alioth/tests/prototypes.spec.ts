/**
 * auth-web-alioth — the console's prototype-only visibility rule.
 *
 * Source is a paid, time-limited download: the console must serve prototypes
 * (`Pre-Proc/{ns}/Prototypes/**`, `Apps/{app}/prototype.html`, an app's private
 * prototype tree) and nothing else. These cases are the rule's contract, and the
 * same allowlist decides both the `/preview/…` byte server and the 原型 listing.
 */
import { describe, expect, it, beforeEach, afterEach } from 'vitest'
import { mkdir, mkdtemp, rm, symlink, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { isPrototypePath, listPrototypes, prototypeUrl } from '../src/prototypes.ts'

let root: string
let preProcRoot: string

beforeEach(async () => {
  root = await mkdtemp(path.join(tmpdir(), 'dsh-alioth-prototypes-'))
  preProcRoot = path.join(root, 'Pre-Proc')
  await mkdir(path.join(preProcRoot), { recursive: true })
})

afterEach(async () => {
  await rm(root, { recursive: true, force: true })
})

async function write(relative: string, contents = 'x'): Promise<void> {
  const file = path.join(root, relative)
  await mkdir(path.dirname(file), { recursive: true })
  await writeFile(file, contents, 'utf8')
}

describe('isPrototypePath', () => {
  it('allows the two sanctioned prototype layouts', () => {
    for (const allowed of [
      'Pre-Proc/U-ada/Prototypes/index.html',
      'Pre-Proc/U-ada/Prototypes/_shared/lifecycle.ts',
      'Pre-Proc/U-ada/Prototypes/Modules/stock/index.html',
      'Pre-Proc/U-ada/Prototypes/Blocks/list/index.html',
      'Pre-Proc/U-ada/Apps/default/prototype.html',
      'Pre-Proc/U-ada/Apps/default/Prototypes/shell.html',
      'Pre-Proc/U-ada/Apps/default/Prototypes/_shared/base.css',
    ]) {
      expect(isPrototypePath(allowed), allowed).toBe(true)
    }
  })

  it('denies source, registry artifacts and traces', () => {
    for (const denied of [
      // Source and its per-module manifests.
      'Pre-Proc/U-ada/Apps/default/Sources/Modules/stock/module.json',
      'Pre-Proc/U-ada/Apps/default/modules/stock/block.json',
      'Pre-Proc/U-ada/Apps/default/Sources/Apps/default/main.rs',
      // App contract + extension declarations + evidence.
      'Pre-Proc/U-ada/Apps/default/app.json',
      'Pre-Proc/U-ada/Apps/default/extensions/rules.yaml',
      'Pre-Proc/U-ada/Apps/default/e2e-report.json',
      'Pre-Proc/U-ada/Apps/default/extension-verify.json',
      'Pre-Proc/U-ada/Apps/default/AppAgentTraces/closure-audit/1.json',
      // Anywhere else in the namespace, and outside Pre-Proc entirely.
      'Pre-Proc/U-ada/Deploy/default/app.json',
      'Pre-Proc/U-ada/Apps',
      'Pre-Proc/U-ada',
      'Deploy/U-ada/x',
      'apps/default/prototype.html',
      // Directory-shaped and traversal-shaped attempts.
      'Pre-Proc/U-ada/Prototypes',
      'Pre-Proc/U-ada/Prototypes/../Apps/default/app.json',
      'Pre-Proc/U-ada/Apps/default/prototype.html/extra',
      'Pre-Proc/U-ada/Apps/default/Prototypes',
      'Pre-Proc/U-ada\\Prototypes\\index.html',
      'Pre-Proc//Prototypes/index.html',
      'Pre-Proc/U-ada/Prototypes/index.html\0.png',
    ]) {
      expect(isPrototypePath(denied), denied).toBe(false)
    }
  })

  it('encodes every path segment when building the preview URL', () => {
    expect(prototypeUrl('Pre-Proc/U-ada/Apps/default/prototype.html'))
      .toBe('/preview/Pre-Proc/U-ada/Apps/default/prototype.html')
    expect(prototypeUrl('Pre-Proc/U-ada/Prototypes/模块 原型/index.html'))
      .toBe('/preview/Pre-Proc/U-ada/Prototypes/%E6%A8%A1%E5%9D%97%20%E5%8E%9F%E5%9E%8B/index.html')
  })
})

describe('listPrototypes', () => {
  it('lists the app entry point first, then the namespace tree', async () => {
    await write('Pre-Proc/U-ada/Apps/default/prototype.html')
    await write('Pre-Proc/U-ada/Prototypes/Modules/stock/index.html')
    await write('Pre-Proc/U-ada/Prototypes/_shared/lifecycle.ts')

    const entries = await listPrototypes({ preProcRoot, namespace: 'U-ada', appCode: 'default' })

    expect(entries.map(entry => entry.rel)).toEqual([
      'Pre-Proc/U-ada/Apps/default/prototype.html',
      'Pre-Proc/U-ada/Prototypes/Modules/stock/index.html',
      'Pre-Proc/U-ada/Prototypes/_shared/lifecycle.ts',
    ])
    expect(entries[0]).toMatchObject({ group: 'app', kind: 'html', label: 'prototype.html' })
    expect(entries[2]).toMatchObject({ group: 'namespace', kind: 'asset', label: '_shared/lifecycle.ts' })
  })

  it('never follows a symlink out of the prototype tree', async () => {
    await write('Pre-Proc/U-ada/Apps/default/Sources/main.rs', 'SECRET')
    await mkdir(path.join(preProcRoot, 'U-ada', 'Prototypes'), { recursive: true })
    // A prototype tree that links back into the source directory must stay inert.
    await symlink(
      path.join(preProcRoot, 'U-ada', 'Apps', 'default', 'Sources'),
      path.join(preProcRoot, 'U-ada', 'Prototypes', 'sneaky'),
    )
    await write('Pre-Proc/U-ada/Prototypes/real.html')

    const entries = await listPrototypes({ preProcRoot, namespace: 'U-ada', appCode: 'default' })

    expect(entries.map(entry => entry.rel)).toEqual(['Pre-Proc/U-ada/Prototypes/real.html'])
    expect(entries.some(entry => entry.rel.includes('main.rs'))).toBe(false)
  })

  it('reports an empty list for an app that has no prototypes yet', async () => {
    await mkdir(path.join(preProcRoot, 'U-ada', 'Apps', 'default'), { recursive: true })

    expect(await listPrototypes({ preProcRoot, namespace: 'U-ada', appCode: 'default' })).toEqual([])
  })
})
