#!/usr/bin/env bun
/**
 * derived-copies.ts — C 类派生副本的**单一声明面**（平台单源 → 目标侧副本）
 *
 * 规约来源:
 *   - openspec/specs/shared-capability-single-source :: derived-copy-manifest-single-source
 *     （C 类四元组的声明面 = 本 manifest；物化端与认证端 MUST 读同一份声明）
 *   - openspec/specs/ns-isolation :: isolation-gate-coverage
 *
 * 为什么要有这一份：此前「派生副本」这个概念的 interface 分散在各处——每族各配一个手写门禁，
 * 或在物化脚本里各写一行 `cp`，或两者都没有。实测后果（2026-10-04）：
 * `Deploy/{ns}/scripts/lib/decrypt-env-values.sh` 与平台单源出现 3 个版本（4/5 ns 缺 keyring
 * previous-key 轮换回退），而它是包内 `start.sh` 的运行期解密路径——零门禁告警。
 *
 * 本模块把「有哪些族 / 源在哪 / 落到哪 / 谁认证 / 物化是否据此」收成一份可枚举的声明，
 * 并把 `certifiedBy` 设为**必填**（无 `ungated` 取值）⇒「无守护的派生副本」在类型上不可表达。
 *
 * 用法:
 *   bun scripts/lib/derived-copies.ts --list
 *   bun scripts/lib/derived-copies.ts --print --ns WZ
 *   bun scripts/lib/derived-copies.ts --materialize --ns WZ --deploy-dir <Deploy/WZ> \
 *       --platform-root <平台仓根> [--standalone]
 *
 * 物化语义与既有 `release-to-namespace.sh` 逐条等价（覆盖写 + 需要时 chmod +x；
 * model-seed 默认相对软链 `../../../Framework/seed`，`--standalone` 时实体复制）。
 * 本模块**不做 prune**（镜像目录对齐由发行链的 rsync/prune 段承担）。
 */
import { chmodSync, copyFileSync, existsSync, lstatSync, mkdirSync, readdirSync, rmSync, symlinkSync, unlinkSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';

/** 副本一致性判据的形态。 */
export type DerivedCopyMode =
  /** 逐字节等于源（单文件）。 */
  | 'byte-equal'
  /** 目录树：`--standalone` 时实体复制、否则相对软链。 */
  | 'seed-tree'
  /** 目标锁文件须覆盖源锁的 (包, 版本) 对（超集语义，MUST NOT 相等）。 */
  | 'tree-superset'
  /** 原型镜像：目标侧 MUST 为指向源（设计目录）的**相对符号链接**（零字节单源）。 */
  | 'prototype-link';

/** 目标路径的基准：平台仓根，或 ns 仓根。 */
export type DerivedCopyBase = 'platform' | 'ns-repo';

export interface DerivedCopy {
  /** 稳定标识（认证输出、测试、日志引用）。 */
  id: string;
  /** 平台单源（平台仓内相对路径；目录面以 `/**` 表示）。 */
  source: string;
  /** 目标路径基准。 */
  destBase: DerivedCopyBase;
  /** 目标模板（`{ns}` 占位；相对 destBase 所指仓根）。 */
  destTemplate: string;
  mode: DerivedCopyMode;
  /** 守护门禁 id——MUST 存在于 `scripts/pre/gates.ts`（覆盖审计断言）。 */
  certifiedBy: string;
  /** 发布/同步链是否据本条目物化。 */
  materialize: boolean;
  /** 物化时是否补可执行位（与既有 release 语义一致）。 */
  exec?: boolean;
  note?: string;
}

/**
 * C 类派生副本清单。
 *
 * `certifiedBy` 取值必须是 `scripts/pre/gates.ts` 中真实登记的 id——「无守护」不可表达。
 * `materialize: true` 的条目由 `--materialize` 物化（当前全部为 `release-to-namespace.sh` 的
 * `cp -f` / `ln -sfn` 面）；只声明不物化的条目（ns 仓派生物，由实例化器/模板通道落盘）仅参与
 * 覆盖审计与各自的既有认证器。
 */
export const DERIVED_COPIES: readonly DerivedCopy[] = [
  {
    id: 'gateway-serve-frontend',
    source: 'scripts/gateway/serve-frontend.sh',
    destBase: 'platform',
    destTemplate: 'Deploy/{ns}/scripts/serve-frontend.sh',
    mode: 'byte-equal',
    certifiedBy: 'ns-deploy-frontend-drift',
    materialize: true,
    exec: true,
    note: '发布包自带静态服务（SPA fallback + /api 代理）',
  },
  {
    id: 'gateway-frontend-server',
    source: 'scripts/gateway/frontend-server.js',
    destBase: 'platform',
    destTemplate: 'Deploy/{ns}/scripts/frontend-server.js',
    mode: 'byte-equal',
    certifiedBy: 'ns-deploy-frontend-drift',
    materialize: true,
    note: '零三方依赖的 node 静态/代理服务器',
  },
  {
    id: 'deploy-system-config-enc-key',
    source: 'scripts/lib/system-config-enc-key.sh',
    destBase: 'platform',
    destTemplate: 'Deploy/{ns}/scripts/lib/system-config-enc-key.sh',
    mode: 'byte-equal',
    certifiedBy: 'check-derived-copies',
    materialize: true,
    note: '独立分发场景的包内 helper（release `cp -f` 语义）',
  },
  {
    id: 'deploy-decrypt-env-values',
    source: 'scripts/lib/decrypt-env-values.sh',
    destBase: 'platform',
    destTemplate: 'Deploy/{ns}/scripts/lib/decrypt-env-values.sh',
    mode: 'byte-equal',
    certifiedBy: 'check-derived-copies',
    materialize: true,
    note: '包内 `start.sh` 的运行期解密路径（含 keyring previous-key 轮换回退）',
  },
  {
    id: 'deploy-seed-login-check',
    source: 'scripts/deploy/seed-login-check.sh',
    destBase: 'platform',
    destTemplate: 'Deploy/{ns}/scripts/seed-login-check.sh',
    mode: 'byte-equal',
    certifiedBy: 'check-derived-copies',
    materialize: true,
    note: '包启动器 HELPER 解析包内优先（无源码树冷启）',
  },
  {
    id: 'deploy-model-seed',
    source: 'Framework/seed',
    destBase: 'platform',
    destTemplate: 'Deploy/{ns}/seed/model-seed',
    mode: 'seed-tree',
    certifiedBy: 'check-derived-copies',
    materialize: true,
    note: '默认相对软链 `../../../Framework/seed`；`--standalone` 实体复制',
  },
  {
    id: 'ns-bootstrap',
    source: 'scripts/dev/ns-bootstrap-template.sh',
    destBase: 'ns-repo',
    destTemplate: '.alioth/bootstrap.sh',
    mode: 'byte-equal',
    certifiedBy: 'ns-bootstrap-consistency',
    materialize: false,
    note: 'ns 仓最早运行的引导脚本（模板 + 逐字节一致性门禁）',
  },
  {
    id: 'ns-host-manifest',
    source: 'scripts/gateway/instantiate-ns-host.ts',
    destBase: 'ns-repo',
    destTemplate: 'Pre-Proc/{ns}/gateway-host/{Cargo.toml,build.rs}',
    mode: 'byte-equal',
    certifiedBy: 'ns-derived-consistency',
    materialize: false,
    note: '组合根实例化器的 `--check` 产出（模板 = Gateway/host）',
  },
  {
    id: 'ns-host-lock',
    source: 'scripts/gateway/instantiate-ns-host.ts',
    destBase: 'ns-repo',
    destTemplate: 'Pre-Proc/{ns}/gateway-host/Cargo.lock',
    mode: 'tree-superset',
    certifiedBy: 'ns-derived-consistency',
    materialize: false,
    note: '模板锁 (包, 版本) 对全含的超集不变量（MUST NOT 相等）',
  },
  {
    id: 'prototype-mirror',
    source: 'Pre-Proc/{ns}/Prototypes/{kind}/<unit>',
    destBase: 'ns-repo',
    destTemplate: 'Pre-Proc/{ns}/Sources/Apps/{kind}/<unit>',
    mode: 'prototype-link',
    certifiedBy: 'check-derived-copies',
    materialize: false,
    note: 'sync-prototype.sh 的镜像段：镜像 MUST 为**指向真相源的相对符号链接**（零字节；漂移结构上不可能）；`prototype.html` 指向设计侧活动版本 html（`module.json#prototypeVersion`）',
  },
];

// ── 物化 ────────────────────────────────────────────────────

export interface MaterializeOptions {
  platformRoot: string;
  ns: string;
  /** `Deploy/{ns}` 的绝对路径（平台仓档位 = `<platformRoot>/Deploy/{ns}`，符号链接指向 ns 仓）。 */
  deployDir: string;
  standalone?: boolean;
  /** 只物化给定族 id（缺省 = 全部 `materialize` 条目）。 */
  ids?: readonly string[];
}

export interface MaterializeAction {
  id: string;
  from: string;
  to: string;
  how: 'copy' | 'copy-tree' | 'symlink';
}

/**
 * 目标绝对路径的**单一解析器**（物化端、认证端、既有 per-family 门禁共用）。
 *   · `destBase === 'platform'` ⇒ `<nsRepoRoot>/Deploy/<ns>/<模板去掉 'Deploy/{ns}/' 前缀>`；
 *   · `destBase === 'ns-repo'`  ⇒ `<nsRepoRoot>/<模板>`。
 * `prototype-pair` 无单一路径（面级关系），调用方 MUST NOT 对其调用本函数。
 */
export function destPathFor(copy: DerivedCopy, ns: string, nsRepoRoot: string): string {
  if (copy.destBase === 'platform') {
    const rel = copy.destTemplate.replace('Deploy/{ns}/', '').replace(/\{ns\}/g, ns);
    return join(nsRepoRoot, 'Deploy', ns, ...rel.split('/'));
  }
  return join(nsRepoRoot, ...copy.destTemplate.replace(/\{ns\}/g, ns).split('/'));
}

/** 计划物化动作（干跑；不触碰磁盘）。规则：目标 = deployDir + 模板去掉 `Deploy/{ns}/` 前缀。 */
export function planMaterialize(opts: MaterializeOptions): MaterializeAction[] {
  const wanted = opts.ids ? new Set(opts.ids) : null;
  const actions: MaterializeAction[] = [];
  for (const copy of DERIVED_COPIES.filter(
    (c) => c.materialize && c.destBase === 'platform' && (wanted === null || wanted.has(c.id)),
  )) {
    const rel = copy.destTemplate.replace('Deploy/{ns}/', '').replace(/\{ns\}/g, opts.ns);
    actions.push({
      id: copy.id,
      from: join(opts.platformRoot, copy.source),
      to: join(opts.deployDir, ...rel.split('/')),
      how: copy.mode === 'seed-tree' ? (opts.standalone ? 'copy-tree' : 'symlink') : 'copy',
    });
  }
  return actions;
}

/** 执行物化（覆盖写语义，与既有 release `cp -f` / `ln -sfn` 逐条等价）。 */
export function materialize(opts: MaterializeOptions): MaterializeAction[] {
  const actions = planMaterialize(opts);
  for (const action of actions) {
    const copy = DERIVED_COPIES.find((c) => c.id === action.id) as DerivedCopy;
    mkdirSync(dirname(action.to), { recursive: true });
    if (!existsSync(action.from)) {
      throw new Error(`[derived-copies] 平台单源缺失：${action.from}（族 ${copy.id}）`);
    }
    switch (action.how) {
      case 'copy':
        copyFileSync(action.from, action.to);
        if (copy.exec) chmodSync(action.to, 0o755);
        break;
      case 'copy-tree':
        rmSync(action.to, { recursive: true, force: true });
        // 实体复制目录树（保序无关；release 的 `cp -R` 语义）
        cpTree(action.from, action.to);
        break;
      case 'symlink': {
        // 既有语义：相对软链 `../../../Framework/seed`（自 Deploy/{ns}/seed/ 上溯 3 级 = 仓根）
        if (existsSync(action.to) || isLink(action.to)) unlinkSync(action.to);
        symlinkSync('../../../Framework/seed', action.to);
        break;
      }
    }
  }
  return actions;
}

function isLink(p: string): boolean {
  try {
    return lstatSync(p).isSymbolicLink();
  } catch {
    return false;
  }
}

function cpTree(from: string, to: string): void {
  mkdirSync(to, { recursive: true });
  for (const entry of readdirSync(from, { withFileTypes: true })) {
    const src = join(from, entry.name);
    const dst = join(to, entry.name);
    if (entry.isDirectory()) cpTree(src, dst);
    else copyFileSync(src, dst);
  }
}

// ── CLI ─────────────────────────────────────────────────────

function flagValue(argv: readonly string[], name: string): string | null {
  const i = argv.indexOf(name);
  return i >= 0 && argv[i + 1] ? argv[i + 1]! : null;
}

function main(): void {
  const argv = process.argv.slice(2);
  if (argv.includes('--list')) {
    for (const c of DERIVED_COPIES) {
      console.log(
        `${c.id}\t${c.mode}\t${c.certifiedBy}\t${c.materialize ? 'materialize' : 'declare-only'}\t${c.source} → ${c.destBase}/${c.destTemplate}`,
      );
    }
    return;
  }

  const ns = flagValue(argv, '--ns');
  const platformRoot = resolve(flagValue(argv, '--platform-root') ?? '.');
  const deployDir = flagValue(argv, '--deploy-dir') ?? (ns ? join(platformRoot, 'Deploy', ns) : null);

  if (argv.includes('--print') || argv.includes('--materialize')) {
    if (!ns || !deployDir) {
      console.error('[derived-copies] 用法：--print|--materialize --ns <ns> [--deploy-dir <dir>] [--platform-root <dir>] [--standalone]');
      process.exit(2);
    }
    const opts: MaterializeOptions = {
      platformRoot,
      ns,
      deployDir: resolve(deployDir),
      standalone: argv.includes('--standalone'),
      ids: flagValue(argv, '--ids')?.split(',').filter(Boolean),
    };
    if (argv.includes('--materialize')) {
      for (const a of materialize(opts)) console.log(`[derived-copies] ${a.how}\t${a.from} → ${a.to}`);
      console.log('[derived-copies] 物化完成');
      return;
    }
    for (const a of planMaterialize(opts)) console.log(`[plan] ${a.how}\t${a.from} → ${a.to}`);
    return;
  }

  console.error('[derived-copies] 用法：--list | --print | --materialize（见文件头注释）');
  process.exit(2);
}

if (import.meta.main) main();
