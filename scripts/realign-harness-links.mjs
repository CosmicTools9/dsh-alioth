// Self-heal for the shared source tree: this workspace lists the harness
// tree as members, so any install here re-points the harness packages'
// react link at THIS workspace's store. Because react-dom is not a direct
// dependency of every harness react consumer, that half-update splits
// React across two physical stores and breaks the harness client test
// lane with dual-instance hook errors. After our installs, re-run the
// harness install (frozen, scripts skipped) so its links resolve from its
// own store again. Best effort: never fails this workspace's install.
import { execSync } from 'node:child_process'
import { existsSync, realpathSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const here = path.dirname(fileURLToPath(import.meta.url))
const harness = path.resolve(here, '../deepseek-harness')
const probe = path.join(harness, 'packages/client/ui-workspace/node_modules/react')

try {
  if (!existsSync(probe)) process.exit(0)
  const real = realpathSync(probe)
  if (real.startsWith(harness + path.sep)) process.exit(0)
  console.warn(`[realign-harness] react resolves outside the harness tree (${real}); re-running the harness install`)
  execSync('pnpm install --frozen-lockfile --ignore-scripts', {
    cwd: harness,
    stdio: 'inherit',
    env: { ...process.env, ALL_PROXY: '', all_proxy: '', HTTP_PROXY: '', http_proxy: '', HTTPS_PROXY: '', https_proxy: '' },
  })
} catch (error) {
  console.warn(`[realign-harness] skipped: ${error?.message ?? error}`)
}
process.exit(0)
