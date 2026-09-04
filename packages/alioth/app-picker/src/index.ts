/**
 * AppCreator workspace picker.
 *
 * The harness web GUI's 「选择工作空间」flow browses the filesystem through
 * the `browse` directory-picker capability and turns a picked directory into
 * the active workspace (ctx.workspaceRegistry.create(path)). For AppCreator
 * the choice is not a folder — it is the managed App the user wants to work
 * on. This package replaces the browse backend (web-profile patch row
 * `directory-picker`) with one whose listing is the Alioth tree
 * `Pre-Proc/<namespace>/Apps/<app>`, so the SPA picker offers namespaces and
 * apps, and picking an app creates the workspace over that app's real
 * directory (the artifact root the AppAgent tools operate on).
 *
 * The package is both backend and assembler: applying it registers the
 * browse capability (ctx.directoryPicker, one per context) and mounts the
 * harness's own browse client surface as a loader entry, mirroring what
 * @deepseek-ai/dsh-host-directory-picker-auto does for its chosen backend.
 *
 * @module @dsh-alioth/app-picker
 */

import { join, resolve } from 'node:path'
import { homedir } from 'node:os'
import { readdir } from 'node:fs/promises'
import type { Context } from '@deepseek-ai/cordis'
import {
  DirectoryPicker,
  DirectoryPickerError,
  type DirectoryEntry,
  type DirectoryListing,
  type DirectoryPickerCapability,
} from '@deepseek-ai/dsh-host-directory-picker'
import type {} from '@deepseek-ai/cordis-plugin-loader'
import z from '@deepseek-ai/schemastery'

/** Alioth artifact root default: env override, else the home convention. */
export function defaultPreProcRoot(): string {
  const env = process.env.ALIOTH_PRE_PROC_ROOT
  return env !== undefined && env !== '' ? resolve(env) : resolve(homedir(), '.dsh-alioth', 'Pre-Proc')
}

/** Backend configuration — deployment choices only. */
export interface Config {
  /** Pre-Proc root whose namespaces/apps are offered (default: env ALIOTH_PRE_PROC_ROOT, else ~/.dsh-alioth/Pre-Proc). */
  preProcRoot?: string
  /** Lock the picker to one namespace (AppCreator standard mode); unset lists every namespace (AppAgent-style). */
  namespace?: string
}

export const Config: z<Config> = z.object({
  preProcRoot: z.string(),
  namespace: z.string(),
})

/** App directories under one namespace's Apps folder, name-sorted. */
async function appDirsOf(namespaceDir: string): Promise<DirectoryEntry[]> {
  const appsRoot = join(namespaceDir, 'Apps')
  let names: string[]
  try {
    names = await readdir(appsRoot, { withFileTypes: true }).then(entries =>
      entries.filter(e => e.isDirectory() && !e.name.startsWith('.')).map(e => e.name).sort())
  } catch {
    // No Apps folder yet (fresh namespace): an empty level, not an error.
    return []
  }
  return names.map(name => ({ name, path: join(appsRoot, name), hidden: false }))
}

/** Namespace directories under the Pre-Proc root, name-sorted. */
async function namespaceDirsOf(preProcRoot: string): Promise<DirectoryEntry[]> {
  let names: string[]
  try {
    names = await readdir(preProcRoot, { withFileTypes: true }).then(entries =>
      entries.filter(e => e.isDirectory() && !e.name.startsWith('.')).map(e => e.name).sort())
  } catch (error) {
    throw new DirectoryPickerError('directory-unreadable', preProcRoot, `cannot list "${preProcRoot}": ${error instanceof Error ? error.message : String(error)}`)
  }
  return names.map(name => ({ name, path: join(preProcRoot, name), hidden: false }))
}

/** The `ctx.directoryPicker` browse implementation over the Alioth tree. */
export class AppDirectoryPicker extends DirectoryPicker {
  static Config: typeof Config = Config

  private readonly root: string
  private readonly namespace: string | undefined
  private readonly browseCapability: DirectoryPickerCapability = {
    kind: 'browse',
    list: (path, signal) => this.list(path, signal),
    createDirectory: (path, name) => this.createDirectory(path, name),
  }

  /** @param ctx - owning Context. */
  constructor(ctx: Context, config: Config) {
    super(ctx)
    this.root = resolve(config.preProcRoot ?? defaultPreProcRoot())
    this.namespace = config.namespace
  }

  /** The browse interaction capability. */
  capability(): DirectoryPickerCapability {
    return this.browseCapability
  }

  /**
   * One listing level of the Alioth tree. The seam contract takes fully
   * qualified paths only; anything outside the Pre-Proc root is refused.
   * Levels: root → namespaces (or the configured one directly) → apps →
   * (leaf, empty — an app's internals are not offered as pickable rows).
   */
  private async list(path?: string, signal?: AbortSignal): Promise<DirectoryListing> {
    const root = this.root
    if (path !== undefined) {
      const target = resolve(path)
      if (target !== root && !target.startsWith(root + '/')) {
        throw new DirectoryPickerError('directory-unreadable', path, `cannot list "${path}": outside the Pre-Proc root`)
      }
    }
    const ns = this.namespace
    if (path === undefined) {
      // Root: the configured namespace's apps directly (AppCreator lock), or
      // every namespace as the first browse level.
      if (ns !== undefined) return this.namespaceListing(ns, signal)
      const entries = await namespaceDirsOf(root)
      if (signal?.aborted === true) throw new DirectoryPickerError('directory-unreadable', root, 'listing aborted')
      return {
        path: root,
        home: root,
        crumbs: [{ name: '工作区', path: root, hidden: false }],
        entries,
        truncated: false,
      }
    }
    const target = resolve(path)
    if (ns !== undefined) {
      const nsDir = join(root, ns)
      if (target === root || target === nsDir) return this.namespaceListing(ns, signal)
      return this.appLeafListing(ns, target, signal)
    }
    // Unconfigured: target is a namespace directory (or inside one).
    const rel = target === root ? '' : target.slice(root.length + 1)
    const nsName = rel.split('/')[0]
    if (nsName === undefined || nsName === '') {
      throw new DirectoryPickerError('directory-unreadable', target, `cannot list "${target}": no namespace segment`)
    }
    if (rel.split('/').length === 1) return this.namespaceListing(nsName, signal)
    return this.appLeafListing(nsName, target, signal)
  }

  /** Namespace-level listing: the namespace's app directories. */
  private async namespaceListing(namespace: string, signal?: AbortSignal): Promise<DirectoryListing> {
    const nsDir = join(this.root, namespace)
    const entries = await appDirsOf(nsDir)
    if (signal?.aborted === true) throw new DirectoryPickerError('directory-unreadable', nsDir, 'listing aborted')
    return {
      path: nsDir,
      home: this.root,
      crumbs: [
        { name: '工作区', path: this.root, hidden: false },
        { name: namespace, path: nsDir, hidden: false },
      ],
      entries,
      truncated: false,
    }
  }

  /** Leaf listing for an app directory: pickable, not browseable. */
  private async appLeafListing(namespace: string, target: string, signal?: AbortSignal): Promise<DirectoryListing> {
    const nsDir = join(this.root, namespace)
    if (signal?.aborted === true) throw new DirectoryPickerError('directory-unreadable', nsDir, 'listing aborted')
    const rel = target.startsWith(nsDir + '/') ? target.slice(nsDir.length + 1) : ''
    const segments = rel.split('/')
    const app = segments[0] === 'Apps' ? segments[1] : segments[0]
    const crumbs: DirectoryEntry[] = [
      { name: '工作区', path: this.root, hidden: false },
      { name: namespace, path: nsDir, hidden: false },
    ]
    if (app !== undefined && app !== '') crumbs.push({ name: app, path: join(nsDir, 'Apps', app), hidden: false })
    return { path: target, home: this.root, crumbs, entries: [], truncated: false }
  }

  /**
   * The browse flow offers a create-directory action; apps are contract
   * artifacts (app.json + extensions) that must be generated programmatically
   * (alioth_app_write), never scaffolded as bare folders — refuse.
   */
  private async createDirectory(path: string, name: string): Promise<string> {
    throw new DirectoryPickerError(
      'directory-create-failed',
      join(path, name),
      'AppCreator apps are contract artifacts — create them in the agent dialogue, not as folders',
    )
  }
}

/**
 * Web-profile plugin row replacing `directory-picker`
 * (@deepseek-ai/dsh-host-directory-picker-auto): registers the App picker
 * backend and mounts the harness browse client surface so the SPA keeps its
 * picker UI while its content becomes the Alioth app tree.
 */
export const name = 'app-picker-alioth'
export const inject = ['loader'] as const

/** Plugin entry (apply is async: loader mounts are awaited). */
export async function apply(ctx: Context, config: Config): Promise<void> {
  const resolved: Config = {
    ...config.preProcRoot === undefined ? {} : { preProcRoot: config.preProcRoot },
    ...config.namespace === undefined ? {} : { namespace: config.namespace },
  }
  await ctx.effect(async () => {
    await ctx.plugin(AppDirectoryPicker, resolved)
    // Mirrors directory-picker-auto: mount the browse client surface after
    // the backend lands (the surface drives the capability at runtime).
    const surfaceId = await ctx.loader.create({ name: '@deepseek-ai/dsh-client-ui-directory-picker-browse' })
    return async () => {
      if (ctx.loader.store[surfaceId] !== undefined) {
        await ctx.loader.remove(surfaceId)
      }
    }
  }, 'app-picker-alioth: app-tree picker entries')
}
