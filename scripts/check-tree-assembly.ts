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
import { spawnSync } from 'node:child_process'
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
}

const ALIOTH_PLUGINS = [
  'env-alioth',
  'tool-alioth',
  'tool-alioth-meta',
  'tool-alioth-workflow',
  'tool-alioth-orchestrator',
] as const

const COMPOSITIONS: readonly Composition[] = [
  {
    name: 'bundle (headless deployment)',
    profile: 'headless',
    patch: 'packages/alioth/bundle-alioth/cordis.patch.yml',
    expectPlugins: ['landing-alioth', 'auth-alioth', 'auth-web-alioth', 'billing-alioth', 'billing-web-alioth', 'feedback-alioth', 'feedback-web-alioth', 'tool-feedback-alioth', ...ALIOTH_PLUGINS],
    expectPersona: 'You are the Alioth app agent',
  },
  {
    name: 'example overlay (headless dev)',
    profile: 'headless',
    patch: 'examples/alioth-agent/cordis.yml',
    expectPlugins: [...ALIOTH_PLUGINS],
    expectPersona: 'You are the Alioth app agent',
  },
  {
    name: 'example web patch (web GUI dev)',
    profile: 'web',
    patch: 'examples/alioth-agent/web.patch.yml',
    expectPlugins: [...ALIOTH_PLUGINS],
    expectPersona: 'Alioth', // web patch only inserts plugins; base persona untouched
  },
  {
    name: 'web + bundle patch (launch GUI)',
    profile: 'web',
    patch: 'packages/alioth/bundle-alioth/cordis.patch.yml',
    expectPlugins: ['landing-alioth', 'auth-alioth', 'auth-web-alioth', 'billing-alioth', 'billing-web-alioth', 'feedback-alioth', 'feedback-web-alioth', 'tool-feedback-alioth', ...ALIOTH_PLUGINS],
    expectPersona: 'You are the Alioth app agent',
  },
]

function run(cmd: string, args: readonly string[]): ReturnType<typeof spawnSync> {
  return spawnSync(cmd, args, { cwd: ROOT, encoding: 'utf8' })
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
