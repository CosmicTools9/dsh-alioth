import { describe, expect, it } from 'vitest'
import { mkdtemp, mkdir, writeFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { Context } from '@deepseek-ai/cordis'
import { DirectoryPickerError, type DirectoryListing } from '@deepseek-ai/dsh-host-directory-picker'
import { AppDirectoryPicker } from '../src/index.ts'

type Browse = (path?: string, signal?: AbortSignal) => Promise<DirectoryListing>

/** Narrow a picker capability to its browse face. */
function browse(picker: AppDirectoryPicker): Browse {
  const capability = picker.capability()
  if (capability.kind !== 'browse') throw new Error('expected browse capability')
  return capability.list
}

/** One picker over a temp Pre-Proc tree with two namespaces and apps. */
async function fixture(namespace?: string): Promise<{ picker: AppDirectoryPicker; root: string; nsTest: string; appA1: string }> {
  const ctx = new Context()
  const root = await mkdtemp(join(tmpdir(), 'app-picker-'))
  const nsTest = join(root, 'U-test')
  const nsOther = join(root, 'U-other')
  await mkdir(join(nsTest, 'Apps', 'a1'), { recursive: true })
  await writeFile(join(nsTest, 'Apps', 'a1', 'app.json'), '{}')
  await mkdir(join(nsTest, 'Apps', 'b2'), { recursive: true })
  await mkdir(join(nsOther, 'Apps', 'x9'), { recursive: true })
  // Non-app directories at the roots must not leak into listings.
  await mkdir(join(nsTest, 'loose-dir'))
  await mkdir(join(root, '.hidden'))
  const appA1 = join(nsTest, 'Apps', 'a1')
  const picker = new AppDirectoryPicker(ctx, { preProcRoot: root, ...namespace === undefined ? {} : { namespace } })
  return { picker, root, nsTest, appA1 }
}

/** Cleanup helper for one fixture. */
async function cleanup(...paths: string[]): Promise<void> {
  for (const p of paths) await rm(p, { recursive: true, force: true })
}

describe('AppDirectoryPicker listing (AppCreator tree)', () => {
  it('lists namespaces at the root with a 工作区 crumb', async () => {
    const f = await fixture()
    const listing = await browse(f.picker)()
    expect(listing.path).toBe(f.root)
    expect(listing.home).toBe(f.root)
    expect(listing.crumbs).toEqual([{ name: '工作区', path: f.root, hidden: false }])
    expect(listing.entries.map(e => e.name)).toEqual(['U-other', 'U-test'])
    expect(listing.entries.every(e => e.hidden === false)).toBe(true)
    expect(listing.truncated).toBe(false)
    await cleanup(f.root)
  })

  it('lists apps under a namespace, skipping loose dirs', async () => {
    const f = await fixture()
    const listing = await browse(f.picker)(f.nsTest)
    expect(listing.path).toBe(f.nsTest)
    expect(listing.entries.map(e => e.name)).toEqual(['a1', 'b2'])
    expect(listing.crumbs.map(c => c.name)).toEqual(['工作区', 'U-test'])
    await cleanup(f.root)
  })

  it('offers an app directory as a pickable leaf (empty listing)', async () => {
    const f = await fixture()
    const listing = await browse(f.picker)(f.appA1)
    expect(listing.entries).toEqual([])
    expect(listing.crumbs.map(c => c.name)).toEqual(['工作区', 'U-test', 'a1'])
    expect(listing.crumbs.at(-1)?.path).toBe(f.appA1)
    await cleanup(f.root)
  })

  it('locks directly onto the configured namespace from the root', async () => {
    const f = await fixture('U-test')
    const listing = await browse(f.picker)()
    expect(listing.path).toBe(f.nsTest)
    expect(listing.entries.map(e => e.name)).toEqual(['a1', 'b2'])
    await cleanup(f.root)
  })

  it('refuses paths outside the Pre-Proc root', async () => {
    const f = await fixture()
    await expect(browse(f.picker)(join(f.root, '..', 'escape'))).rejects.toBeInstanceOf(DirectoryPickerError)
    await cleanup(f.root)
  })

  it('refuses folder creation (apps are contract artifacts)', async () => {
    const f = await fixture()
    const capability = f.picker.capability()
    if (capability.kind !== 'browse') throw new Error('expected browse capability')
    await expect(capability.createDirectory(f.nsTest, 'scratch')).rejects.toBeInstanceOf(DirectoryPickerError)
    await cleanup(f.root)
  })
})
