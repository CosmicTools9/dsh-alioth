/**
 * ns-topology.ts — 平台 ↔ namespace 拓扑的**唯一 module**（ns 集合 / 端口槽位 / 共享面集合 / 面四元组登记）
 *
 * 规约来源：`openspec/specs/shared-capability-single-source`
 *   （`ns-topology-single-source`：拓扑事实 MUST 由唯一 module 持有；
 *    `sharing-mode-classification`：每面 MUST 归入三类之一并声明四元组，四元组的守护门禁 MUST 存在且可执行）。
 *
 * 为什么需要（2026-10-05 实测）：同一拓扑事实曾以 5 份互不引用的字面量存在，且两处注释**自称同源而实际不同**
 *   —— `scripts/global-search/ns-catalog.ts:23`（`NS_LIST`，注释自称「Deploy/ports.conf 的 5 槽位」）；
 *   `scripts/lib/shared-face.ts`（`SHARED_FACE_PREFIXES` 含 `.husky`，注释自称「同源常量 = CANONICAL_FACES」）；
 *   `scripts/check/check-ns-shared-surface-registry.ts`（`CANONICAL_FACES` 不含 `.husky`）。
 *   两份面集合**语义确实不同**（`.husky` 是平台专有面、MUST NOT 进 ns 仓 `LINKS`），但差异此前只存在于注释里。
 *
 * 单一事实源的分工（不新造数据）：
 *   · **部署/端口真相源** = `Deploy/ports.conf`（既有；`scripts/lib/ns-ports.sh` 自述的单一事实源）。
 *     本 module **复用既有唯一读取器** `table-code-registry::portSlotsFromPortsConf`（MUST NOT 新造第二实现），
 *     只在其上做「行 → `NsSlot`」的严格化（端口列非数字 ⇒ 记为 problem，由门禁 fail-loud）。
 *   · **编译期类型面** = `NS_NAMES`（本 module 的元组常量）：供类型与生成器使用；
 *     其与 `Deploy/ports.conf` 行的**集合一致性由门禁断言**（`check-ns-topology.ts`）——两者不一致即阻断。
 *   · **面集合** = `FACE_REGISTRY`（`mounted` / `portalOnly` 两个布尔字段表达两族差异），
 *     `facesMountedToNs()` / `sharedFacePatterns()` 是它的两个**派生视图**，消费者只读视图。
 */

import { portSlotsFromPortsConf } from './table-code-registry';

/** ns 集合的**编译期**面孔（顺序 = `Deploy/ports.conf` 行序）；与表行的集合一致性由 `check-ns-topology.ts` 断言。 */
export const NS_NAMES = ['Alioth', 'Cosmic-Tools', 'WZ', 'AVIC-CAASEC', 'SE'] as const;
export type NsName = (typeof NS_NAMES)[number];

/** 一个 ns 的端口槽位（`Deploy/ports.conf` 的一行）。 */
export interface NsSlot {
  ns: string;
  fe: number;
  be: number;
  sso: number;
}

/** 面分型（`sharing-mode-classification` 的三类）。 */
export type SharingClass = 'A' | 'B' | 'C';

/** 共享面登记（四元组）：`guardGates` 各项 MUST 是 `scripts/pre/gates.ts` 中真实登记且可执行的 gate id。 */
export interface FaceRecord {
  /** 面名（相对平台仓根的顶层条目名）。 */
  face: string;
  /** 是否进 ns 仓 `.alioth/platform.conf` 的 `LINKS`（ns 以目录级符号链接消费该整面）。 */
  mounted: boolean;
  /** 仅门户 ns 挂载（判据 = 该 ns 的 `Open/Apps/<app>/app.json` 存在）。 */
  portalOnly: boolean;
  class: SharingClass;
  /** 单源位置（四元组①）。 */
  singleSource: string;
  /** 变更通道（四元组②）。 */
  changeChannel: string;
  /** 守护门禁（四元组③）——id 存在性由 `check-ns-topology.ts` 断言。 */
  guardGates: readonly string[];
  /** 回滚面（四元组④）。 */
  rollback: string;
}

/**
 * 面登记表（**唯一的**面集合声明面）。
 *
 * 分型依据（依序三问，见 `sharing-mode-classification`）：① 内容真相源在平台仓 ⇒ 属本表；
 * ② ns 仓只读不写 ⇒ **A 类**（本表全部为 A 类：只读工具/源码面；B/C 类由 `DERIVED_COPIES` 与
 * baselines 台账承载，不在面表内）。`Meta` 面的用途 MUST 限于「ns 仓内运行的门禁取
 * `schema-info`/`meta-backend` 二进制」——与 ADR D-035 修订后的「ns MUST NOT `path` 依赖 `Meta/backend/*`」
 * 是两条不同的面（挂载面 vs 编译依赖面），并列而不冲突。
 */
export const FACE_REGISTRY: readonly FaceRecord[] = [
  {
    face: '.agents',
    mounted: true,
    portalOnly: false,
    class: 'A',
    singleSource: '.agents/（平台仓）',
    changeChannel: '平台仓提交；ns 仓经 LINKS 目录级符号链接即时消费',
    guardGates: ['shared-face-annotation', 'ns-face-smoke', 'skill-adapter-sync'],
    rollback: 'git revert 平台提交（ns 仓零副本，无需同步）',
  },
  {
    face: '.omp',
    mounted: true,
    portalOnly: false,
    class: 'A',
    singleSource: '.omp/（平台仓）',
    changeChannel: '平台仓提交；ns 仓经 LINKS 消费',
    guardGates: ['shared-face-annotation', 'ns-face-smoke'],
    rollback: 'git revert 平台提交',
  },
  {
    face: 'scripts',
    mounted: true,
    portalOnly: false,
    class: 'A',
    singleSource: 'scripts/（平台仓）',
    changeChannel: '平台仓提交；ns 仓经 LINKS 消费（含 ns 仓 hook 逻辑单源 `scripts/pre/ns-hook.sh`）',
    guardGates: ['shared-face-annotation', 'ns-face-smoke', 'repo-wide-gates', 'gate-path-exempt'],
    rollback: 'git revert 平台提交',
  },
  {
    face: 'Framework',
    mounted: true,
    portalOnly: false,
    class: 'A',
    singleSource: 'Framework/（平台仓；面内自带 `[workspace]` 根 manifest）',
    changeChannel: '平台仓提交；ns 经 LINKS 源码依赖（只读，ADR D-045）',
    guardGates: ['shared-face-annotation', 'face-dependency-closure', 'check-derived-copies'],
    rollback: 'git revert 平台提交（ns 侧零副本）',
  },
  {
    face: 'Meta',
    mounted: true,
    portalOnly: false,
    class: 'A',
    singleSource: 'Meta/（平台仓；面内自根 = `Meta/backend/Cargo.toml`）',
    changeChannel: '平台仓提交；ns 侧用途**仅限**门禁取 `schema-info`/`meta-backend` 二进制（编译依赖面为禁，见 D-035 修订）',
    guardGates: ['face-dependency-closure', 'ns-shared-surface-registry'],
    rollback: 'git revert 平台提交',
  },
  {
    face: 'Gateway',
    mounted: true,
    portalOnly: false,
    class: 'A',
    singleSource: 'Gateway/（平台仓；`host/` = 组合根模板单一来源）',
    changeChannel: '平台仓提交；ns 仓实例化组合根后经 LINKS 使用框架 lib',
    guardGates: ['shared-face-annotation', 'platform-ns-agnostic', 'ns-derived-consistency'],
    rollback: 'git revert 平台提交 + 各 ns 重跑 `instantiate-ns-host.ts --check`',
  },
  {
    face: 'SSO',
    mounted: true,
    portalOnly: false,
    class: 'A',
    singleSource: 'SSO/（平台仓）',
    changeChannel: '平台仓提交；ns 仓经 LINKS 消费（形态由 `Deploy/sso-mode.conf` 决定）',
    guardGates: ['shared-face-annotation', 'ns-derived-consistency'],
    rollback: 'git revert 平台提交',
  },
  {
    face: 'vendor',
    mounted: true,
    portalOnly: false,
    class: 'A',
    singleSource: 'vendor/（平台仓；根 `Cargo.toml` 的 `[patch.crates-io]` 引用）',
    changeChannel: '平台仓提交；ns 仓经 LINKS 消费',
    guardGates: ['repo-wide-gates'],
    rollback: 'git revert 平台提交',
  },
  {
    face: 'OpenActivity',
    mounted: true,
    portalOnly: true,
    class: 'A',
    singleSource: 'OpenActivity/（平台仓）',
    changeChannel: '平台仓提交；**仅门户 ns** 经 LINKS 消费（判据 = `Open/Apps/<app>/app.json` 存在）',
    guardGates: ['platform-ns-agnostic', 'ns-derived-consistency'],
    rollback: 'git revert 平台提交',
  },
  {
    face: '.husky',
    mounted: false,
    portalOnly: false,
    class: 'A',
    singleSource: '.husky/（平台仓薄壳 → `scripts/pre/`）',
    changeChannel: '平台仓提交。**平台专有面**：MUST NOT 进 ns 仓 `LINKS`（ns 仓 hook 生效面 = `.alioth/hooks`）',
    guardGates: ['gate-hook-wiring', 'shared-face-annotation'],
    rollback: 'git revert 平台提交',
  },
];

/** ns 仓 `LINKS` 的期望面集（非门户 ns）；门户 ns 另加 `portalOnly` 面。 */
export function facesMountedToNs(): string[] {
  return FACE_REGISTRY.filter((r) => r.mounted && !r.portalOnly).map((r) => r.face);
}

/** 仅门户 ns 挂载的面。 */
export function portalFaces(): string[] {
  return FACE_REGISTRY.filter((r) => r.mounted && r.portalOnly).map((r) => r.face);
}

/**
 * 平台共享面的**路径前缀集**（A 类变更标注 / ns 面 smoke 的扫描面）= 挂载面 ∪ 平台专有面。
 *
 * 消费者 MUST 同源（禁第二套前缀表）：`scripts/pre/gates.ts`（gate `scope`）、
 * `scripts/check/check-shared-face-annotation.ts`（`[shared-face]` 标注判定 + `--classify`）。
 */
export function sharedFacePrefixes(): readonly string[] {
  return FACE_REGISTRY.map((r) => r.face);
}

/** 前缀 → 正则模式串（`scope` 与 checker 的 `RegExp` 构造共用）。 */
export function sharedFacePatterns(): string[] {
  return sharedFacePrefixes().map(
    (prefix) => `^${prefix.replace(/[.*+?^${}()|[\]\\]/g, '\\$&').replace(/\//g, '\\/')}\\/`,
  );
}

/**
 * 读取 `Deploy/ports.conf` 并**严格化**为 `NsSlot`（复用 `table-code-registry` 的既有唯一读取器；
 * 端口列非数字与空表记入 `problems`，由门禁 fail-loud）。ns 仓档位经 `portsTableRoot` 解析平台仓根。
 */
export function nsSlots(root: string): { rows: NsSlot[]; problems: string[] } {
  const problems: string[] = [];
  const rows: NsSlot[] = [];
  for (const slot of portSlotsFromPortsConf(root)) {
    if (slot.fe === null || slot.be === null || slot.sso === null) {
      problems.push(`ns ${slot.ns} 的端口列非纯数字（表损坏）：${slot.fe}/${slot.be}/${slot.sso}`);
      continue;
    }
    rows.push({ ns: slot.ns, fe: slot.fe, be: slot.be, sso: slot.sso });
  }
  if (rows.length === 0) problems.push('Deploy/ports.conf 不可达或空表（判据面失效，MUST NOT 视作通过）');
  return { rows, problems };
}

/** 登记的 ns 名清单（推导自表；表不可用 ⇒ 空数组，调用方 MUST fail-loud）。 */
export function nsList(root: string): string[] {
  return nsSlots(root).rows.map((r) => r.ns);
}

