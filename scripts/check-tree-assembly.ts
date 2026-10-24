/**
 * Gate: the shipped composition trees assemble cleanly.
 *
 * Runs `dsh --dump-config` for every documented composition and asserts:
 *   - exit 0
 *   - every expected plugin row is present in the assembled tree
 *     (patch `insert:` rows with unmatched ids are silently dropped by dsh,
 *      so a missing row means a broken patch)
 *   - the system-prompt persona was replaced (top-level patch rows REPLACE
 *     existing rows — a stale id silently no-ops)
 *   - the shared-online closure held: harness rows disabled by the bundle
 *     patch are still disabled, and pinned row values did not drift
 *     (expectDisabled / expectRowText)
 *   - no warning lines anywhere in the output
 *
 * Compositions:
 *   headless + bundle patch   — the canonical deployment group (auth incl.)
 *   headless + example overlay — dev composition (auth-free by design:
 *                               single-user dialogue; auth is the bundle's
 *                               B/S deployment concern)
 *   web + example web patch    — browser GUI dev composition (tools only)
 *   web + bundle patch         — `mise run launch` composition (full B/S
 *                               surface: auth badge, billing, feedback)
 *
 * Self-contained: re-runs scripts/link-dsh-profiles.sh first (idempotent) so
 * the dsh profile fallback resolves @dsh-alioth/* packages.
 * Usage: node --import tsx scripts/check-tree-assembly.ts
 */
import { spawnSync, type SpawnSyncReturns } from 'node:child_process'
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import path from 'node:path'

const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url))
const ROOT = path.resolve(SCRIPT_DIR, '..')
const DSH = path.join(ROOT, 'node_modules', '.bin', 'dsh')

interface Composition {
  readonly name: string
  readonly profile: string
  readonly patch: string
  /** Plugin ids that must appear in the assembled tree. */
  readonly expectPlugins: readonly string[]
  /** Text that must appear in the persona (patch replaced the row). */
  readonly expectPersona: string
  /**
   * Manifest that must DECLARE every `@dsh-alioth/*` plugin this composition
   * mounts. Mounting is by package name, so an undeclared row silently drops
   * out of the package's `pnpm deploy` closure and any consumer that derives
   * the plugin set from the manifest — re-found by hand, hence asserted here.
   * Harness rows (`@deepseek-ai/*`) are exempt: they resolve through the
   * profile fallback, which `link-dsh-profiles.sh` owns.
   */
  readonly declaringManifest?: string
  /** The deployment patch must not offer danger-full-access (完全控制).
   * Only the two bundle compositions set this: the example overlays are the
   * single-user dev shape (no auth, no shared space) and keep base defaults. */
  readonly forbidFullAccess?: boolean
  /**
   * Harness rows that must stay DISABLED in the assembled tree — the shared
   * online-surface closure from the 2026-10-23 security assessment. Patch
   * disables are matched by id, so an upstream rename or row drop turns the
   * disable into a silent no-op and the capability silently returns; this is
   * the tripwire. Only rows the composition's own tree contains may be listed
   * (web-only rows are unmatched skips on headless).
   */
  readonly expectDisabled?: readonly string[]
  /**
   * Row id → text that must appear inside that row's assembled section
   * (fail-closed pins against upstream default flips, e.g. session full-text
   * search staying `openAt: never`).
   */
  readonly expectRowText?: Readonly<Record<string, readonly string[]>>
  /**
   * EXACT allowlist of enabled (not-disabled) row ids — the shared-online
   * closure as a whitelist, not a denylist: an upstream row that mounts a new
   * capability goes red here until it is consciously assessed and listed.
   * Both bundle compositions pin their full enabled set.
   */
  readonly expectEnabledRows?: readonly string[]
}

const ALIOTH_PLUGINS = [
  'env-alioth',
  'tool-alioth',
  'tool-alioth-meta',
  'tool-alioth-workflow',
  'tool-alioth-orchestrator',
  'guard-alioth',
  'tool-alioth-verify',
] as const

const COMPOSITIONS: readonly Composition[] = [
  {
    name: 'bundle (headless deployment)',
    profile: 'headless',
    patch: 'packages/alioth/bundle-alioth/cordis.patch.yml',
    declaringManifest: 'packages/alioth/bundle-alioth/package.json',
    expectPlugins: ['landing-alioth', 'auth-alioth', 'auth-web-alioth', 'billing-alioth', 'billing-web-alioth', 'page-feedback', 'feedback-web-alioth', 'tool-feedback-alioth', ...ALIOTH_PLUGINS],
    expectPersona: 'You are the Alioth app agent',
    forbidFullAccess: true,
    // Base-layer rows the shared-online closure disables in every profile
    // (web-only rows are unmatched skips here and are asserted on web only).
    expectDisabled: ['plugin-manager', 'tool-plugin-manager', 'web-deepseek-api'],
    // Full enabled-row allowlist: any newly mounted harness row goes red here
    // until the shared-online security assessment consciously lists it.
    expectEnabledRows: [
      'agent', 'agent-default-model', 'agent-instructions',
      'agent-loop', 'approval', 'attachment-local',
      'auth-alioth', 'auth-web-alioth', 'bash-sandbox',
      'billing-alioth', 'billing-web-alioth', 'command-compact',
      'command-feedback', 'command-goal', 'commands',
      'compaction-basic', 'config-editor', 'deepseek-llm-api-extensions',
      'directory-picker-app', 'env-alioth', 'feedback-web-alioth',
      'fs-observation-policy', 'fs-sandbox', 'goal',
      'goal-round-driver', 'guard-alioth', 'headless-runner',
      'headless-startup', 'image-offload', 'jobs',
      'landing-alioth', 'llm', 'llm-alioth',
      'llm-deepseek-account', 'llm-pi-ai', 'llm-retry',
      'mcp-resources', 'otel', 'page-feedback',
      'permission', 'plan-mode', 'plugin-package-inventory-deepseek',
      'ptc-runtime', 'pwsh-sandbox', 'remote-access',
      'repeat-tool-reminder', 'sandbox', 'sandbox-policy',
      'session', 'session-checkpoint-policy', 'session-log-deepseek',
      'session-persistence-jsonl', 'session-projection', 'session-projection-cache',
      'session-query-sqlite', 'session-telemetry-otel', 'session-title',
      'session-title-llm', 'settings', 'shell-env',
      'skill', 'skill-filesystem', 'spill-local',
      'spill-policy', 'storage', 'storage-domain',
      'storage-json', 'subagent', 'subagent-fork-in-process',
      'subagent-spawn-in-process', 'subprocess', 'system-prompt',
      'timeout-policy', 'timer', 'token-meter',
      'tool-alioth', 'tool-alioth-meta', 'tool-alioth-orchestrator',
      'tool-alioth-verify', 'tool-alioth-workflow', 'tool-bash',
      'tool-feedback-alioth', 'tool-fs', 'tool-fs-search',
      'tool-goal', 'tool-jobs', 'tool-pwsh',
      'tool-result-pruner', 'tool-skill', 'tool-subagent',
      'tool-subagent-control', 'tool-subagent-fork', 'tool-subagent-list-agents',
      'tool-todo', 'tool-web', 'tool-workflow',
      'tool-working-directory', 'tools', 'typert',
      'typert-gateway', 'typert-loader', 'user-questions',
      'web', 'web-fetch-http', 'web-search-deepseek',
      'workflow-ptc', 'working-directory',
    ],
    expectRowText: {
      // The generic-file upload cap is pinned (bytes), not left to the
      // upstream default, so a silent default flip cannot reopen disk fill.
      'attachment-local': ['maxFileBytes: 67108864'],
    },
  },
  {
    name: 'example overlay (headless dev)',
    profile: 'headless',
    patch: 'examples/alioth-agent/cordis.yml',
    declaringManifest: 'examples/alioth-agent/package.json',
    expectPlugins: [...ALIOTH_PLUGINS],
    expectPersona: 'You are the Alioth app agent',
  },
  {
    name: 'example web patch (web GUI dev)',
    profile: 'web',
    patch: 'examples/alioth-agent/web.patch.yml',
    declaringManifest: 'examples/alioth-agent/package.json',
    expectPlugins: [...ALIOTH_PLUGINS],
    // web patch only inserts plugins; the base persona (harness default
    // "coding agent") stays untouched — assert its marker, not an Alioth one.
    expectPersona: 'You are a coding agent',
  },
  {
    name: 'web + bundle patch (launch GUI)',
    profile: 'web',
    patch: 'packages/alioth/bundle-alioth/cordis.patch.yml',
    declaringManifest: 'packages/alioth/bundle-alioth/package.json',
    expectPlugins: ['landing-alioth', 'auth-alioth', 'auth-web-alioth', 'billing-alioth', 'billing-web-alioth', 'page-feedback', 'feedback-web-alioth', 'tool-feedback-alioth', ...ALIOTH_PLUGINS],
    expectPersona: 'You are the Alioth app agent',
    forbidFullAccess: true,
    // The 2026-10-23 shared-online closure (see the bundle patch comments for
    // the per-row rationale). plugin-manager/tool-plugin-manager/web-deepseek-
    // api are base rows and are already asserted on headless.
    expectDisabled: [
      'settings-controller', 'ui-settings-web-search', 'ui-settings-account',
      'account-controller', 'session-log-download', 'schedule', 'ui-schedule',
      'office-to-pdf', 'workspace-changes', 'harness-dev-context',
      'preset-cordis', 'preset-minimal', 'ui-plugin-manager',
    ],
    expectRowText: {
      // Full-text session search spans every session on the host — it must
      // stay opt-in-off (cross-tenant content search would defeat isolation).
      'session-query-sqlite': ['openAt: never'],
      // The browser tab stays profile-gated off on web: the disable must
      // still be the profile gate (upstream flipping it on, or removing the
      // row, both go red).
      'ui-sidebar-browser': ['disabled:', 'profileContext'],
      // The two host-wide session verbs stay off (desktop open / default-model
      // write); see the 2026-10-23 residual-risk closure in the bundle patch.
      'session-controller': ['openWorkspacePath: false', 'initializeDefaultModel: false'],
      'attachment-local': ['maxFileBytes: 67108864'],
    },
    // Full enabled-row allowlist (see the headless entry for the rationale).
    expectEnabledRows: [
      'agent', 'agent-default-model', 'agent-loop',
      'agent-preset-registry', 'api-remotes', 'approval',
      'attachment-local', 'auth-alioth', 'auth-web-alioth',
      'authorization', 'bash-sandbox', 'billing-alioth',
      'billing-web-alioth', 'client-hmr', 'command-feedback',
      'commands', 'config-editor', 'connection',
      'cordis-client-runner', 'cordis-host-runner', 'cordis-inspect-providers',
      'credentials', 'deepseek-account', 'deepseek-llm-api-extensions',
      'desktop-product-telemetry', 'directory-picker-app', 'env-alioth',
      'feedback-web-alioth', 'file-reference-local', 'file-upload',
      'fs-observation-policy', 'fs-sandbox', 'goal',
      'goal-round-driver', 'guard-alioth', 'hmr',
      'image-offload', 'job-controller', 'jobs',
      'landing-alioth', 'llm', 'llm-alioth',
      'llm-deepseek-account', 'llm-pi-ai', 'llm-retry',
      'locale', 'mcp-resources', 'message-feedback',
      'modules', 'otel', 'page-feedback',
      'permission', 'plugin-inventory', 'plugin-package-inventory-deepseek',
      'preset-ptc', 'preset-standard', 'product-analytics',
      'ptc-runtime', 'pwsh-sandbox', 'remote-access',
      'repeat-tool-reminder', 'resources', 'sandbox',
      'sandbox-policy', 'session', 'session-checkpoint-policy',
      'session-controller', 'session-log-deepseek', 'session-persistence-jsonl',
      'session-projection', 'session-projection-cache', 'session-query-sqlite',
      'session-reference', 'session-stats', 'session-telemetry-otel',
      'session-title', 'session-title-llm', 'session-turn-outline',
      'settings', 'shell-env', 'shortcuts',
      'skill', 'spill-local', 'spill-policy',
      'storage', 'storage-domain', 'storage-json',
      'subagent', 'subagent-fork-in-process', 'subagent-model-selection-settings',
      'subagent-spawn-in-process', 'subprocess', 'system-prompt',
      'timeout-policy', 'timer', 'token-meter',
      'tool-alioth', 'tool-alioth-meta', 'tool-alioth-orchestrator',
      'tool-alioth-verify', 'tool-alioth-workflow', 'tool-feedback-alioth',
      'tool-working-directory', 'tools', 'typert',
      'typert-gateway', 'typert-loader', 'ui-agent-preset',
      'ui-approval', 'ui-attachment', 'ui-brand-official',
      'ui-chat', 'ui-commands', 'ui-conversation',
      'ui-cordis', 'ui-deepseek-model-label-1017', 'ui-goal',
      'ui-input-trigger', 'ui-jobs', 'ui-layout',
      'ui-message-feedback', 'ui-model-selection', 'ui-permission',
      'ui-plan', 'ui-reference', 'ui-renderer',
      'ui-secret-mask', 'ui-session', 'ui-settings',
      'ui-settings-agent-loop', 'ui-settings-general', 'ui-settings-models',
      'ui-settings-plugin-inventory', 'ui-settings-plugins', 'ui-settings-session-log',
      'ui-settings-shell', 'ui-settings-subagent', 'ui-shortcuts',
      'ui-sidebar', 'ui-sidebar-browser', 'ui-sidebar-right',
      'ui-skill', 'ui-subagent', 'ui-theme',
      'ui-tool', 'ui-trajectory', 'ui-user-questions',
      'ui-workflow-run', 'ui-workspace', 'user-questions',
      'web', 'web-fetch-http', 'web-runtime',
      'web-search-deepseek', 'web-startup', 'webserver',
      'working-directory', 'workspace', 'workspace-controller',
    ],
  },
]

/**
 * Every row id in the assembled tree that is not literally `disabled: true`.
 * Rows whose disable is a `!!js` expression count as enabled — the expression
 * evaluates at boot, and a dump cannot prove it false, so the allowlist
 * treats "not proven disabled" as mounted. That is the conservative side.
 */
function enabledRowsOf(out: string): string[] {
  const sections = out.split('\n- id: ')
  const enabled: string[] = []
  for (let index = 1; index < sections.length; index++) {
    const section = sections[index]
    if (section === undefined) continue
    const id = section.split('\n', 1)[0]?.trim()
    if (id !== undefined && id !== '' && !/^ {2}disabled: true$/m.test(section)) enabled.push(id)
  }
  return enabled.sort()
}

function run(cmd: string, args: readonly string[]): SpawnSyncReturns<string> {
  return spawnSync(cmd, args, { cwd: ROOT, encoding: 'utf8' })
}

/** One entry section of the dumped config (`- id: X` up to the next row). */
function rowOf(out: string, id: string): string | undefined {
  const start = out.indexOf(`- id: ${id}\n`)
  if (start === -1) {
    return undefined
  }
  const next = out.indexOf('\n- id: ', start + 1)
  return out.slice(start, next === -1 ? undefined : next)
}

function main(): void {
  // Self-contained: make the profile fallback resolve our packages.
  const link = run('bash', [path.join(SCRIPT_DIR, 'link-dsh-profiles.sh')])
  if (link.status !== 0) {
    console.error(`✗ link-dsh-profiles.sh failed:\n${link.stdout ?? ''}${link.stderr ?? ''}`)
    process.exitCode = 1
    return
  }

  const problems: string[] = []
  for (const comp of COMPOSITIONS) {
    const res = run(DSH, ['--profile', comp.profile, '--patch', comp.patch, '--dump-config'])
    const out = `${res.stdout ?? ''}`
    const err = `${res.stderr ?? ''}`
    const label = `${comp.name} [${comp.profile} + ${comp.patch}]`

    if (res.status !== 0) {
      problems.push(`${label}: dsh exited ${res.status}\n${out}${err}`)
      continue
    }
    for (const plugin of comp.expectPlugins) {
      if (!out.includes(`- id: ${plugin}`)) {
        problems.push(`${label}: plugin row \`${plugin}\` missing from assembled tree (patch insert row dropped or renamed)`)
      }
      // --dump-config prints each entry as `- id: X` + two-space `name: …`.
      // A patch row whose import key is wrong (e.g. `plugin:` instead of
      // `name:`) still shows its id here but lacks the name line — and the
      // loader fails at real boot. Assert the pair, not just the id.
      if (!out.includes(`- id: ${plugin}\n  name: `)) {
        problems.push(`${label}: plugin row \`${plugin}\` has no \`name:\` import key in the assembled tree (loader fails at boot)`)
      }
    }
    if (!out.includes(comp.expectPersona)) {
      problems.push(`${label}: persona marker "${comp.expectPersona}" not found (system-prompt row not patched)`)
    }
    if (comp.forbidFullAccess === true) {
      const permission = rowOf(out, 'permission')
      if (permission === undefined) {
        problems.push(`${label}: permission row missing from the assembled tree`)
      } else {
        if (permission.includes('danger-full-access')) {
          problems.push(`${label}: permission row still offers danger-full-access (完全控制) — a shared online space must not expose it`)
        }
        if (!permission.includes('workspace-write')) {
          problems.push(`${label}: permission row lost the workspace-write preset`)
        }
      }
      const policy = rowOf(out, 'sandbox-policy')
      if (policy === undefined || !/^\s+mode: workspace-write$/m.test(policy)) {
        problems.push(`${label}: sandbox-policy.mode is not pinned to workspace-write (patch row dropped or env drift)`)
      }
    }
    for (const id of comp.expectDisabled ?? []) {
      const row = rowOf(out, id)
      if (row === undefined) {
        problems.push(`${label}: expected-disabled row \`${id}\` not found in the assembled tree (upstream renamed or dropped the row — re-run the shared-online security assessment before trusting this closure)`)
      } else if (!/^ {2}disabled: true$/m.test(row)) {
        problems.push(`${label}: row \`${id}\` is no longer disabled in the assembled tree — the shared-online closure regressed (upstream default flip or lost patch row)`)
      }
    }
    if (comp.expectEnabledRows !== undefined) {
      const enabled = enabledRowsOf(out)
      const pinned = [...comp.expectEnabledRows].sort()
      const unexpected = enabled.filter(id => !pinned.includes(id))
      const missing = pinned.filter(id => !enabled.includes(id))
      if (unexpected.length > 0) {
        problems.push(`${label}: enabled rows outside the pinned allowlist (assess before listing): ${unexpected.join(', ')}`)
      }
      if (missing.length > 0) {
        problems.push(`${label}: allowlisted rows not enabled in the tree: ${missing.join(', ')}`)
      }
    }
    for (const [id, needles] of Object.entries(comp.expectRowText ?? {})) {
      const row = rowOf(out, id)
      if (row === undefined) {
        problems.push(`${label}: row \`${id}\` missing from the assembled tree (expected it to carry ${needles.map(n => `\`${n}\``).join(' / ')})`)
        continue
      }
      for (const needle of needles) {
        if (!row.includes(needle)) {
          problems.push(`${label}: row \`${id}\` lost \`${needle}\` in the assembled tree`)
        }
      }
    }
    if (comp.declaringManifest !== undefined) {
      const manifest = JSON.parse(readFileSync(path.join(ROOT, comp.declaringManifest), 'utf8')) as {
        dependencies?: Record<string, string>
      }
      const declared = new Set(Object.keys(manifest.dependencies ?? {}))
      const mounted = [...out.matchAll(/^  name: '(@dsh-alioth\/[^']+)'$/gm)].map(match => match[1] as string)
      for (const pkg of new Set(mounted)) {
        if (!declared.has(pkg)) {
          problems.push(`${label}: \`${pkg}\` is mounted but not declared in ${comp.declaringManifest} — add it to dependencies (mounting is by name; the manifest is the declared plugin set)`)
        }
      }
    }
    const warningLines = (out + err)
      .split('\n')
      .filter(line => /warn/i.test(line))
    if (warningLines.length > 0) {
      problems.push(`${label}: warning lines in dump:\n${warningLines.join('\n')}`)
    }
  }

  if (problems.length > 0) {
    for (const p of problems) console.error(`✗ ${p}`)
    console.error(`\ntree-assembly gate: ${problems.length} violation(s) over ${COMPOSITIONS.length} compositions`)
    process.exitCode = 1
    return
  }
  console.log(`tree-assembly gate: OK (${COMPOSITIONS.length} compositions assemble, all plugin rows + persona present, zero warnings)`)
}

main()
