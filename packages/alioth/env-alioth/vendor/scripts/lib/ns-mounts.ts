/**
 * ns-mounts.ts — ns 面挂载的**单一实现**（symlink-aware 枚举 + 面形态判据）
 *
 * 规约来源：openspec/specs/shared-capability-single-source
 *   （capability `shared-capability-single-source`：共享面判定必须单一实现，禁各门禁各写一份）
 *
 * 背景：`Pre-Proc/{ns}` 是指向 ns 仓的符号链接 ⇒ `Dirent.isDirectory()` 对其返回 false
 * （lstat 语义）——2026-09-26 实测：按 isDirectory 枚举会让 ns 面门禁「静默空转」。
 * 本模块提供两个判据供各门禁复用：
 *   · `mountedNs(root)`：平台仓档位下枚举挂载的 ns 目录（含符号链接）；
 *   · `nsRepoOf(nsDir)`：面目录 → ns 仓根（要求形态 = `<nsRepo>/Pre-Proc/<ns>`）。
 */
import { existsSync, readdirSync, readFileSync, realpathSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';

/**
 * `.alioth/platform.conf` 键值读取——**TS 侧唯一实现**（bash 侧正本 = scripts/lib/ns-ports.sh /
 * ns-sso-mode.sh）。此前各消费点（portsTableRoot 的 META_DIR、instantiate-ns-host 的 SSO_MODE、
 * check-sso-mode-conf 的声明面比对）各持一份键值行切分，语义放宽不会同步（2026-10-11 双线收敛）。
 * 键值行逐字符 indexOf 切分（NO_REGEX_FOR_PARSING：bash 键值面无标准解析器）。
 * 返回 null = 文件缺失/未声明该键（调用方 MUST fail-loud，不猜默认）。
 */
export function platformConfValue(confPath: string, key: string): string | null {
  if (!existsSync(confPath)) return null;
  for (const raw of readFileSync(confPath, 'utf8').split('\n')) {
    const s = raw.trim();
    if (s === '' || s.startsWith('#')) continue;
    const eq = s.indexOf('=');
    if (eq > 0 && s.slice(0, eq) === key) return s.slice(eq + 1).trim();
  }
  return null;
}

/**
 * `Deploy/ports.conf`（ns 清单 / 端口槽位唯一事实源）所在根——**双档位单一实现**。
 *
 *   · 平台仓档位：`<root>/Deploy/ports.conf` 存在 ⇒ `root`；
 *   · ns 仓档位：ns 仓不持有该表（`Deploy/{ns}` 是包目录）⇒ 经 `.alioth/platform.conf` 的
 *     `META_DIR`（平台仓相对位置）解析平台仓根；缺该声明时回落 `scripts` 符号链接 realpath 的父目录。
 * 均不可达 ⇒ `null`（调用方 MUST fail-loud，MUST NOT 静默判通过）。
 */
export function portsTableRoot(root: string): string | null {
  if (existsSync(join(root, 'Deploy', 'ports.conf'))) return root;
  const candidates: string[] = [];
  const confPath = join(root, '.alioth', 'platform.conf');
  const metaDir = platformConfValue(confPath, 'META_DIR');
  if (metaDir) candidates.push(resolve(root, metaDir));
  const scriptsLink = join(root, 'scripts');
  if (existsSync(scriptsLink)) {
    try {
      candidates.push(resolve(realpathSync(scriptsLink), '..'));
    } catch {
      /* 断链：忽略该候选 */
    }
  }
  for (const c of candidates) if (existsSync(join(c, 'Deploy', 'ports.conf'))) return c;
  return null;
}

/** 挂载的 ns 面目录（Platform 档位；ns 档位调用方用 `join(ROOT,'Pre-Proc',own)`）。 */
export function mountedNs(root: string): string[] {
  const dir = join(root, 'Pre-Proc');
  if (!existsSync(dir)) return [];
  return readdirSync(dir, { withFileTypes: true })
    .filter((e) => (e.isDirectory() || e.isSymbolicLink()) && !e.name.startsWith('.'))
    .filter((e) => existsSync(join(dir, e.name, 'Cargo.toml')) || existsSync(join(dir, e.name, 'gateway-host')))
    .map((e) => join(dir, e.name))
    .sort();
}

/** 面目录 → ns 仓根；形态不符（非 `<nsRepo>/Pre-Proc/<ns>`）即抛错（调用方 fail-loud）。 */
export function nsRepoOf(nsDir: string): string {
  const real = realpathSync(nsDir);
  const parts = real.split('/');
  if (parts[parts.length - 2] !== 'Pre-Proc') {
    throw new Error(`ns 面未按契约挂载（应为符号链接 → <nsRepo>/Pre-Proc/{ns}）：${nsDir} → ${real}`);
  }
  return dirname(dirname(real));
}
