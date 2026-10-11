/**
 * check-interface.ts — 检查基板的**唯一 interface**（change `deepen-check-substrate`，C1）。
 *
 * 目的：检查曾普遍自解析 argv、自持基线比较、各自手写（symlink 语义不一致的）目录遍历，并依赖在
 * **import 期**读 `process.argv` 的旧共享层 ⇒ 无法在不伪造 argv 的情况下被测试。本模块把「一条检查」
 * 的取值面收成下述三段式；取值面（根 / 域 / 遍历 / 读行 / 报告 / 基线）唯一经此，检查内 MUST NOT 另建实现。
 *
 * 本模块把「一条检查」的 interface 收成三段式：
 *
 *   defineCheck({ id, baseline?, scopeValue?, predicate(ctx), selfTest })
 *
 * substrate 掌：argv 解析 / 文件域（staged·diff·显式，**fail-closed**）/ 遍历（文件与 symlink 感知的
 * 目录级枚举）/ 报告 / 退出码 / 棘轮 / 自测接线。检查内 **MUST NOT** 读 `process.argv`。
 *
 * 语义（change design §D3，全收敛）：
 *   · `selfTest` **必填**（缺失 ⇒ 注册即抛）；执行注册面（stage/scope/heavy/needsDb）归 gate 表
 *   · git 取暂存集失败 ⇒ **fail-closed**（旧 `gate-kit.stagedFiles` 为 fail-open 返回 `[]`）
 *   · `--cached` ≡ `--staged` 且**蕴含 `--fail`**（`gate-kit:19` 的语义统一到此）
 *   · 基线登记旗标单一 = `--update-baseline`；输入限定旗标单一 = `--scope <值>`
 *     （检查层 `--only` 拒收并提示；执行器层 `run-gates --only` 不受影响）
 */

import {
  existsSync,
  readFileSync,
  readdirSync,
  realpathSync,
  statSync,
} from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { isExcludedDirName } from "./gate-kit";
import * as baseline from "./baseline";
import * as baselineStore from "./baseline-store";

/** 判据返回的违规条目（结构化：检查只描述事实，不决定退出码/输出格式）。 */
export interface Violation {
  /** 仓库相对路径（或逻辑位置标识） */
  path: string;
  /** 单行结论 */
  message: string;
  /** 明细（可选） */
  detail?: string;
  /** 所属通道（声明了 `channels` 时必填；单通道检查省略） */
  channel?: string;
}

/** 谓词拿到的上下文（注入式：检查不自己找仓库根、不自己取暂存集）。 */
export interface CheckContext {
  /** 仓库根（由 substrate 解析；检查 MUST NOT 从 argv/cwd 推断） */
  root: string;
  /**
   * 文件域**请求模式**：`none` = 未请求域（检查应自取整树）；`staged`/`diff` = 已请求，
   * 此时 `files` 即使为空也 MUST 按「域内无文件」判定（≠ 整树）。
   * 缺此字段会让「域为空」与「未请求」不可区分——旧 `CACHED ? stagedFiles() : walkFiles()` 的迁移陷阱。
   */
  fileDomainMode: "none" | "staged" | "diff";
  /** 文件域内容：`fileDomainMode === 'none'` 时为空数组 */
  files: readonly string[];
  /** 检查自有位置参数（基板旗标已剥离；未声明旗标已 fail-closed） */
  argv: readonly string[];
  /** 已声明的自有布尔旗标（`CheckDeclaration.flags` 命中者） */
  flags: ReadonlySet<string>;
  /**
   * 已声明的自有**取值**旗标（`CheckDeclaration.valueFlags` 命中者）→ 取值列表（**按出现顺序**，可重复）。
   * 取值被解析器消费 ⇒ 不落 `argv`（`--file a --file b` ⇒ `['a','b']`，`argv` 不含 a/b）。
   */
  valueFlags: ReadonlyMap<string, readonly string[]>;
  /** `--scope` 取值（未声明 `scopeValue` 的检查得到 null） */
  scopeValue: string | null;
  /** 读仓库内文本（越界路径抛错） */
  readText(rel: string): string;
  /** 文件遍历（排除目录集单源 = `gate-kit.EXCLUDE_DIRS`；不跟符号链接；**按路径排序**保证确定性） */
  walk(ext: readonly string[]): string[];
  /**
   * 「仓库内容」视图：同 `walk`，但只保留**其归属仓已跟踪**的文件（`git ls-files`）。
   *
   * 用途 = ratchet 类判据（存量基线 + 只拦新增）的整树回退域。ratchet 的语义是「仓库内容 vs 基线」，
   * 未入库的临时/在途文件不属于仓库 ⇒ MUST NOT 被算作新增违规（否则任何会话的草稿都会阻断全仓推送）。
   * 归属仓判定：路径实体自深向浅取最近的含 `.git` 祖先（`Pre-Proc/{ns}` 挂载各 ns 仓，平台不跟踪其内容）。
   */
  walkTracked(ext: readonly string[]): string[];
  /** 目录级枚举：符号链接目录**计入结果但不递归**（`Pre-Proc/{ns}` 挂载的既有语义）；**按路径排序** */
  walkDirs(): string[];
  /**
   * 逐行扫描（**注入式**：读经 `readText` ⇒ 可用夹具单测；替代各检查自写的 `扫描循环`）。
   * `skip(line, lineNo)` 返回 true 则跳过该行（注释行 / 元描述行等）。
   */
  scan(
    rel: string,
    pattern: RegExp,
    skip?: (line: string, lineNo: number) => boolean,
  ): Violation[];
}

export interface BaselineDeclaration {
  /** 基线文件（**仓库相对路径**，如 `scripts/check/.foo-baseline.txt`）——IO 仍经 baseline-store */
  file: string;
  /** 规约引用（R2：JSON 基线 SHOULD 带 rule_reference；此处必填） */
  ruleReference: string;
  /** 违规条目的稳定键（棘轮只减不增的判据） */
  keyOf(v: Violation): string;
}

/**
 * **报告通道**：同一检查内的多条规则，各自有 label / hints / baseline / 阻断语义。
 * 旧实现常见形态 = ①硬阻断（无基线） + ②棘轮（有基线）两条 `report/ratchet` 调用；
 * 声明 `channels` 后，违规按 `Violation.channel` 分组，**按声明顺序**逐通道报告（保字节序）。
 * 与顶层 `label/hints/baseline/enforcement` 互斥。
 */
export interface CheckChannel {
  /** 通道标识（`predicate` 给违规打标用） */
  id: string;
  label: string;
  hints?: readonly string[];
  baseline?: BaselineDeclaration;
  enforcement?: "on-fail" | "always";
}

/** 自定义报告面拿到的结果（`report` 缝：检查自绘 stdout）。 */
export interface CheckReportResult {
  /** 本通道全部违规（含存量） */
  violations: readonly Violation[];
  /** 扣掉基线后的新增违规（`--fail`/`always` 判定的依据） */
  fresh: readonly Violation[];
  /** 被基线豁免的条数 */
  grandfathered: number;
  /** `--fail`（含 `--cached` 蕴含）——旧检查自绘时按它切换 [FAIL]/[AUDIT] 等输出分支 */
  fail: boolean;
  /** `--json`——检查若自带 JSON 契约（已发布输出面）由其自行处理 */
  json: boolean;
}

/**
 * 报告流写入面（`report` 缝第三参）：stdout/stderr **双流**，逐字承载旧 `console.log`/`console.error`/`console.warn`。
 * 调用方自行补 `\n`（`console.*` 的换行语义由此显式化）。
 */
export interface CheckIo {
  out(s: string): void;
  err(s: string): void;
}

/** 默认 `io` 实现 = 进程标准流。 */
export function stdIo(): CheckIo {
  return {
    out: (s) => void process.stdout.write(s),
    err: (s) => void process.stderr.write(s),
  };
}

export interface CheckDeclaration {
  id: string;
  /**
   * 检查自身的**判定语义**字段（以下均有运行期消费）。
   *
   * **执行注册面**（何时运行 / 运行阶段 / 重载级别 / DB 依赖）MUST NOT 在此声明——唯一 owner =
   * gate 表 `scripts/pre/gates.ts`（`gate.scope` / `gate.stage` / `gate.cls` / `gate.needsDb`，由
   * `scripts/pre/run-gates.ts` 消费）。检查声明内的同名字段无消费，会与 gate 表静默漂移。
   */
  baseline?: BaselineDeclaration;
  /** 声明并受理 `--scope <值>`（值语义由检查文档化） */
  scopeValue?: "path" | "block-json" | "module-id" | "change-id" | "ns";
  /**
   * 阻断语义：`on-fail`（默认）= 仅 `--fail`/`--cached` 时有违规才 rc=1（审计模式恒 0）；
   * `always` = 有违规即 rc=1（不看 `--fail`）。
   * 迁移前的检查多为模块级 `if (violations.length > 0) process.exit(1)`（无条件阻断），
   * MUST 显式声明为 `always`，否则迁移会静默丢失阻断强度（等价硬件只在夹具含违规时才能发现）。
   */
  enforcement?: "on-fail" | "always";
  /**
   * 文件域语义（迁移保形的关键声明）：
   *   · `requested`（默认）= 只有 `--staged`/`--cached`/`--diff` 才给域；否则空域（检查自取整树）
   *   · `staged` = **总是**给暂存集（旧 `stagedFiles()` 类检查：其实现无视模式恒读暂存区，
   *     且常伴 `enforcement: 'always'`）——迁移 MUST 声明 `staged`，否则默认审计模式下行为改变
   *     （等价硬件实测：`check-forbidden-staged-paths` 默认审计旧 rc=1 / 迁移 rc=0）
   */
  domain?: "requested" | "staged";
  /** 报告用规则名（默认 = `id`）；迁移时**逐字沿用旧 `report()` 的规则文本**以保证 stdout 等价 */
  label?: string;
  /** 违规时的修复提示行（旧 `report()` 的 `修复：…` 行），逐字沿用 */
  hints?: readonly string[];
  /**
   * 检查自有的**布尔旗标**（如 `--dump-baseline`）：声明后由 `ctx.flags` 暴露；
   * 未声明者一律 fail-closed（接口 MUST NOT 静默忽略未知旗标）。
   */
  flags?: readonly string[];
  /**
   * 检查自有的**取值旗标**（如 `--file <p>`，**可重复**）：声明后由 `ctx.valueFlags` 暴露 `出现顺序` 取值列表；
   * 每处出现 MUST 紧跟一个非旗标取值（缺失即抛）。与 `flags` 互斥，且不得复用基板旗标名。
   */
  valueFlags?: readonly string[];
  /**
   * 旗标钩子：命中已声明旗标时、在报告/退出码之前调用。返回 number = 以该码退出（跳过常规报告）；
   * 返回 void/undefined = 继续常规流程。承载「私有旗标改变输出形态」的既有语义
   * （如 `--dump-baseline` 打印键清单并 exit 0）。
   */
  onFlag?(
    flag: string,
    ctx: CheckContext,
    violations: readonly Violation[],
  ): number | void;
  /** 多通道（每条规则独立 label/hints/baseline/enforcement）；与顶层单通道字段互斥 */
  channels?: readonly CheckChannel[];
  /**
   * **自定义报告面**（迁移缝，2026-10-04）：声明后**跳过**基板的标准报告与 `--json` 输出，
   * 由检查自绘 stdout（旧 `console.log` 定制格式的等价承载）。返回 `number` = 以该码退出（覆盖基板判定）；
   * 返回 `void` = 仍按 `fresh.length > 0 && (--fail || enforcement === 'always')` 判定，但不打印基板格式。
   * 第三参 `io` = **stdout/stderr 双流**写入面（`console.*` 的逐字等价承载，调用方自行补 `\n`；
   * 使 stderr 报告的旧检查可等价迁移——2026-10-04 实测：149/204 未迁移检查卡在 stderr 面）。
   * 与 `channels` 互斥（多通道检查的报告面归通道）。
   */
  report?(
    ctx: CheckContext,
    result: CheckReportResult,
    io: CheckIo,
  ): number | void;
  predicate(ctx: CheckContext): Violation[] | Promise<Violation[]>;
  /** 判据自证（R1：**必填**） */
  /** 判据自证（必填）：可返回 `void`/`number`，亦可返回 Promise（判定核为 async 的检查用） */
  selfTest(): void | number | Promise<void | number>;
}

export interface ParsedCheckArgs {
  fail: boolean;
  json: boolean;
  updateBaseline: boolean;
  selfTest: boolean;
  staged: boolean;
  diff: string | null;
  scopeValue: string | null;
  positional: readonly string[];
  /** 命中的已声明自有旗标 */
  flags: ReadonlySet<string>;
  /** 命中的已声明**取值**旗标 → 取值列表（按出现顺序） */
  valueFlags: Map<string, string[]>;
}

const BASE_FLAGS = [
  "--fail",
  "--json",
  "--update-baseline",
  "--self-test",
  "--staged",
  "--cached",
] as const;

/** 仓库根（相对本模块位置；MUST NOT 依赖调用方 cwd） */
export function substrateRoot(): string {
  return resolve(import.meta.dirname, "..", "..", "..");
}

/**
 * 参数解析（同步、纯；导出以便单测与复用）。任何未声明旗标 ⇒ 抛错（fail-closed）。
 */
export function parseCheckArgs(
  decl: Pick<CheckDeclaration, "id" | "scopeValue" | "flags" | "valueFlags">,
  argv: readonly string[],
): ParsedCheckArgs {
  const out: ParsedCheckArgs = {
    fail: false,
    json: false,
    updateBaseline: false,
    selfTest: false,
    staged: false,
    diff: null,
    scopeValue: null,
    positional: [],
    flags: new Set<string>(),
    valueFlags: new Map<string, string[]>(),
  };
  const declaredFlags = new Set(decl.flags ?? []);
  const declaredValueFlags = new Set(decl.valueFlags ?? []);
  const positional: string[] = [];
  for (let i = 0; i < argv.length; i += 1) {
    const a = argv[i]!;
    if (a === "--fail") out.fail = true;
    else if (a === "--json") out.json = true;
    else if (a === "--update-baseline") out.updateBaseline = true;
    else if (a === "--self-test") out.selfTest = true;
    else if (a === "--staged") out.staged = true;
    else if (a === "--cached") {
      // 旧 `gate-kit:19` 语义：`--cached` 蕴含 FAIL ⇒ 统一在此（唯一实现）
      out.staged = true;
      out.fail = true;
    } else if (a === "--diff") {
      out.diff = argv[++i] ?? null;
      if (out.diff === null) throw new Error(`${decl.id}: --diff 缺区间取值`);
    } else if (a === "--scope") {
      if (decl.scopeValue === undefined)
        throw new Error(`${decl.id}: 未声明 scopeValue，不接受 --scope`);
      out.scopeValue = argv[++i] ?? null;
      if (out.scopeValue === null || out.scopeValue === "")
        throw new Error(`${decl.id}: --scope 缺取值`);
    } else if (a === "--only") {
      throw new Error(
        `${decl.id}: 检查层 --only 已收敛为 --scope（执行器层 run-gates --only 不受影响）——` +
          "正本 openspec/specs/precommit-execution；接口说明 docs/specs/SPEC_ENFORCEMENT_SUBSTRATE.md",
      );
    } else if (a.startsWith("--") && declaredValueFlags.has(a)) {
      const v = argv[i + 1];
      if (v === undefined || v.startsWith("--")) {
        throw new Error(
          `${decl.id}: 取值旗标 ${a} 缺取值（MUST 紧跟一个非旗标取值；可重复出现）`,
        );
      }
      i += 1;
      const arr = out.valueFlags.get(a) ?? [];
      arr.push(v);
      out.valueFlags.set(a, arr);
    } else if (a.startsWith("--")) {
      if (declaredFlags.has(a)) {
        out.flags = new Set([...out.flags, a]);
        continue;
      }
      throw new Error(
        `${decl.id}: 未声明的旗标 ${a}（基板旗标 = ${BASE_FLAGS.join(" / ")} / --diff / --scope；` +
          "检查自有旗标 MUST 在 defineCheck 的 flags 中显式声明）",
      );
    } else positional.push(a);
  }
  out.positional = positional;
  return out;
}

/**
 * 文件域（**fail-closed**）：`--staged`/`--cached` 取索引、`--diff` 取区间、都无 ⇒ 空数组（无域输入）。
 * 与旧 `gate-kit.stagedFiles()` 的差别 = git 失败时**抛错**而非静默返回 `[]`。
 */
export function fileDomain(
  root: string,
  parsed: Pick<ParsedCheckArgs, "staged" | "diff">,
): readonly string[] {
  // 文件域 = **提交仓**的索引域（契约见 docs/specs/SPEC_ENFORCEMENT_SUBSTRATE.md；openspec change
  // fix-gate-staged-domain-root）。ns 仓经反向符号链接消费平台 `scripts/`：本模块位置派生的 `root` 是
  // **平台检出**，而索引属于 ns 仓 ⇒ `git -C <平台检出> diff --cached` 会拿外仓索引去平台对象库解析
  // （对象缺失 ⇒ 退出 128 ⇒ fail-closed 误阻断 ns 仓的每一次提交）。提交仓根由门禁执行器
  // （`scripts/pre/run-gates.ts`）经 `ALIOTH_GATE_ROOT` 显式注入；未注入 ⇒ 回退 `root`（平台自身
  // 提交/审计路径行为逐字不变）。
  const domainRoot = (process.env["ALIOTH_GATE_ROOT"] ?? "").trim() || root;
  const args = parsed.staged
    ? ["diff", "--cached", "--name-only", "--diff-filter=ACMR"]
    : parsed.diff !== null
      ? ["diff", "--name-only", "--diff-filter=ACMR", parsed.diff]
      : null;
  if (args === null) return [];
  const r = Bun.spawnSync({
    cmd: ["git", "-C", domainRoot, ...args],
    stdout: "pipe",
    stderr: "pipe",
  });
  if (r.exitCode !== 0) {
    throw new Error(
      `文件域不可判定（git -C ${domainRoot} ${args.join(" ")} 退出 ${r.exitCode}）：${r.stderr.toString().trim()}` +
        " —— 按 fail-closed 处理（判定输入缺失 ≠ 无违规）；取域 root MUST 为提交仓" +
        "（执行器注入 ALIOTH_GATE_ROOT；手工直调时请自行设置）",
    );
  }
  return r.stdout
    .toString()
    .split("\n")
    .map((s) => s.trim())
    .filter((s) => s !== "");
}

/** 归属仓（自深向浅最近的含 `.git` 祖先）：`Pre-Proc/{ns}` 各挂载是独立仓 ⇒ MUST 按归属仓判跟踪。 */
const repoCache = new Map<string, string | null>();
function owningRepo(real: string): string | null {
  let d = dirname(real);
  const chain: string[] = [];
  for (;;) {
    if (repoCache.has(d)) {
      const hit = repoCache.get(d) ?? null;
      for (const c of chain) repoCache.set(c, hit);
      return hit;
    }
    chain.push(d);
    if (existsSync(join(d, ".git"))) {
      for (const c of chain) repoCache.set(c, d);
      return d;
    }
    const parent = dirname(d);
    if (parent === d) {
      for (const c of chain) repoCache.set(c, null);
      return null;
    }
    d = parent;
  }
}

const trackedCache = new Map<string, Set<string> | null>();
function trackedSetOf(repo: string): Set<string> | null {
  if (trackedCache.has(repo)) return trackedCache.get(repo) ?? null;
  const r = Bun.spawnSync({
    cmd: ["git", "-C", repo, "ls-files", "-z"],
    stdout: "pipe",
    stderr: "pipe",
  });
  const set =
    r.exitCode === 0
      ? new Set(
          r.stdout
            .toString()
            .split("\0")
            .filter((s) => s !== ""),
        )
      : null; // 非 git 仓 ⇒ 不猜（调用方 fail-closed 处理）
  trackedCache.set(repo, set);
  return set;
}

/**
 * 仓库内路径（`rel`，posix）是否被**其归属仓**跟踪。用于「仓库内容」口径：
 * ratchet 基线是仓库内容的快照 ⇒ 未入库的在途文件（会话草稿 / 负例夹具）MUST NOT 计作违规。
 */
export function isTrackedPath(root: string, rel: string): boolean {
  let real: string;
  try {
    real = realpathSync(join(root, rel));
  } catch {
    return false;
  }
  const repo = owningRepo(real);
  if (repo === null) return false;
  const set = trackedSetOf(repo);
  return set !== null && set.has(relative(repo, real));
}

/**
 * 目录级枚举的唯一实现（文件与 symlink 感知；排除目录集单源 = `gate-kit.EXCLUDE_DIRS`；按路径排序保证确定性）。
 * `ctx.walk` 与之共用本函数；需**自持 root** 的调用方（如以夹具根驱动自测的检查）亦 MUST 经此取用，
 * MUST NOT 另写一份遍历（第二实现会与基座语义静默漂移）。
 */
export function walkRepo(root: string, ext: readonly string[]): string[] {
  const allowed = new Set(ext.map((e) => e.replace(/^\./, "")));
  const out: string[] = [];
  const ancestors = new Set<string>();
  const visit = (dir: string): void => {
    // **祖先链**环守卫（非全局去重）：同一目录经两条不同路径可达时旧 `walkFiles` 各记一次
    // （实测：全局 realpath 去重会静默漏 966 份经 `Blocks/*/frontend` 符号链接可达的文件）。
    // 只挡「自己是自己的祖先」这一环 ⇒ 无环树结果与旧实现逐字相同，同时不死循环。
    let real: string;
    try {
      real = realpathSync(dir);
    } catch {
      return;
    }
    if (ancestors.has(real)) return;
    ancestors.add(real);
    for (const name of readdirSync(dir)) {
      if (isExcludedDirName(name)) continue;
      const abs = join(dir, name);
      let st;
      try {
        // `statSync`（跟随符号链接）= 旧 `gate-kit.walkFiles` 语义：`Pre-Proc/{ns}` 挂载的
        // ns 内容**在扫描面内**（`lstatSync` 会静默漏掉整片 ns 树——实测：dashboard-page 的
        // 基线存量在主树命中 2 处、在跳过符号链接的实现下 0 处）。
        st = statSync(abs);
      } catch {
        continue;
      }
      if (st.isDirectory()) {
        visit(abs);
        continue;
      }
      const rel = abs.slice(root.length + 1);
      if (st.isFile() && allowed.has(name.split(".").pop() ?? ""))
        out.push(rel);
    }
    ancestors.delete(real);
  };
  visit(root);
  // 排序 = 确定性（旧 `gate-kit.walkFiles` 亦 `out.sort()`；顺序会影响违规输出字节）
  return out.sort();
}

export function buildContext(
  root: string,
  parsed: ParsedCheckArgs,
): CheckContext {
  return {
    root,
    fileDomainMode: parsed.staged
      ? "staged"
      : parsed.diff !== null
        ? "diff"
        : "none",
    files: fileDomain(root, parsed),
    argv: parsed.positional,
    flags: parsed.flags,
    valueFlags: parsed.valueFlags,
    scopeValue: parsed.scopeValue,
    readText(rel) {
      const abs = resolve(root, rel);
      if (!abs.startsWith(`${root}/`)) throw new Error(`越界读取 ${rel}`);
      // 容忍不存在（旧 `gate-kit.readLines` 语义）：文件域可能含「已暂存删除」的路径，
      // 此时「读不到」≠ 判据失败。检查如需区分，用 existsSync 自行判定。
      // 同理容忍**非普通文件**（目录 / 目录软链——如 ns 面内的逐成员平台面软链）：
      // `readFileSync` 对目录抛 EISDIR，会把整条门禁打成崩溃（而非判违规）。
      try {
        if (!statSync(abs).isFile()) return "";
      } catch {
        return "";
      }
      return readFileSync(abs, "utf8");
    },
    walk(ext) {
      return walkRepo(root, ext);
    },
    walkTracked(ext) {
      const out: string[] = [];
      for (const rel of this.walk(ext)) {
        if (isTrackedPath(root, rel)) out.push(rel);
      }
      // `walk` 已排序（确定性）；过滤保持相对顺序 ⇒ 结果逐字确定
      return out;
    },
    walkDirs() {
      const out: string[] = [];
      const ancestors = new Set<string>();
      const visit = (dir: string): void => {
        let real: string;
        try {
          real = realpathSync(dir);
        } catch {
          return;
        }
        if (ancestors.has(real)) return; // 祖先链环守卫（同 walk 语义）
        ancestors.add(real);
        for (const name of readdirSync(dir)) {
          if (isExcludedDirName(name)) continue;
          const abs = join(dir, name);
          let st;
          try {
            st = statSync(abs); // 跟随符号链接（与 walk 同语义）
          } catch {
            continue;
          }
          if (st.isDirectory()) {
            out.push(abs.slice(root.length + 1));
            visit(abs);
          }
        }
        ancestors.delete(real);
      };
      visit(root);
      return out.sort();
    },
    scan(rel, pattern, skip) {
      const out: Violation[] = [];
      const lines = readFileSyncOrEmpty(root, rel).split("\n");
      for (let i = 0; i < lines.length; i += 1) {
        const line = lines[i]!;
        if (skip !== undefined && skip(line, i + 1)) continue;
        if (pattern.test(line)) {
          out.push({
            path: `${rel}:${i + 1}`,
            message: line.trim().slice(0, 120),
            detail: line.trim().slice(0, 120),
          });
        }
      }
      return out;
    },
  };
}

/** 读文件（不存在 ⇒ 空串；与 ctx.readText 同语义，供 scan 复用） */
function readFileSyncOrEmpty(root: string, rel: string): string {
  const abs = resolve(root, rel);
  if (!existsSync(abs)) return "";
  // 同 `ctx.readText`：目录 / 目录软链不是文件，读之抛 EISDIR ⇒ 一律按「读不到」处理
  try {
    if (!statSync(abs).isFile()) return "";
  } catch {
    return "";
  }
  return readFileSync(abs, "utf8");
}

/**
 * **R1 夹具上下文**（判据自证缝）：把内存文件表包成 `CheckContext`，使 `predicate` 可直测而无需
 * 触碰磁盘 / argv / cwd。各检查的 `selfTest()` MUST 用它（避免在 200+ 检查里复制同一段假 ctx）。
 */
export function fixtureContext(
  files: Readonly<Record<string, string>>,
  opts: {
    domain?: readonly string[];
    flags?: readonly string[];
    dirs?: readonly string[];
  } = {},
): CheckContext {
  const domain = opts.domain ?? [];
  return {
    root: "/nonexistent-fixture-root",
    fileDomainMode: domain.length > 0 ? "staged" : "none",
    files: domain,
    argv: [],
    flags: new Set(opts.flags ?? []),
    scopeValue: null,
    readText: (rel) => files[rel] ?? "",
    walk: (ext) =>
      Object.keys(files).filter((f) => ext.some((e) => f.endsWith(e))),
    // 夹具即「仓库内容」：fixtureContext 注入的文件按定义全部视为已跟踪（与 walk 同结果）。
    walkTracked: (ext) =>
      Object.keys(files).filter((f) => ext.some((e) => f.endsWith(e))),
    walkDirs: () => [...(opts.dirs ?? [])],
    scan: (rel, pattern, skip) =>
      (files[rel] ?? "").split("\n").flatMap((line, i) =>
        (skip !== undefined && skip(line, i + 1)) || !pattern.test(line)
          ? []
          : [
              {
                path: `${rel}:${i + 1}`,
                message: line.trim().slice(0, 120),
                detail: line.trim().slice(0, 120),
              },
            ],
      ),
  };
}

export interface RegisteredCheck {
  decl: CheckDeclaration;
  /** CLI 入口：解析 → 域 → 谓词 → 报告/棘轮 → 退出码 */
  main(argv?: readonly string[]): Promise<number>;
}

/**
 * 阻断判定**唯一实现**（报告缝 / 硬阻断早退 / 末段汇总三处同源消费）。
 * `enforcement: 'always'` 无视 `--fail`（旧无条件 `process.exit(1)` 类检查的承载）。
 */
export function blockedBy(
  freshCount: number,
  fail: boolean,
  enforcement?: "on-fail" | "always",
): boolean {
  return freshCount > 0 && (fail || enforcement === "always");
}

/**
 * 旧 `gate-kit.report()` 输出格式的**逐字复刻**（迁移缝：供「旧 gate-kit 消费者」在 `report` 钩子内自绘）。
 *
 * 已有 80 份检查消费旧 gate-kit：其 stdout 为 `  <file>:<line>  <text>`（**含行号、不截断**），而基板
 * 通道渲染是 `  <path>  <detail>`（120 截断）⇒ 无法逐字复刻。本助手把该格式单点收敛，避免 80 份各自
 * 手抄格式（手抄即漂移源）。
 *
 * 位置口径：`v.path` 已含 `:行号` 者原样使用；否则补 `:1`（旧 gate-kit 消费者多为文件级判定，行号恒 1）。
 * 返回 `true` = 旧实现会 `process.exit(1)`（旧 `FAIL = --fail || --cached`；基板 `result.fail` 同义——
 * `--cached` 在基板蕴含 `--fail`）。调用方据此决定钩子返回码（早退顺序由调用方复刻）。
 */
export function legacyGateKitReport(
  rule: string,
  violations: readonly Violation[],
  hints: readonly string[],
  io: CheckIo,
  fail: boolean,
): boolean {
  if (violations.length === 0) {
    io.out(`[OK] ${rule}: 无违规\n`);
    return false;
  }
  io.out(`[VIOLATION] ${rule}: ${violations.length} 处\n`);
  for (const v of violations.slice(0, 50)) {
    const loc = /:\d+$/.test(v.path) ? v.path : `${v.path}:1`;
    io.out(`  ${loc}  ${v.detail ?? v.message}\n`);
  }
  if (violations.length > 50) io.out(`  … 另 ${violations.length - 50} 处\n`);
  for (const h of hints) io.out(`修复：${h}\n`);
  return fail;
}

/**
 * 旧 `gate-kit.ratchet()` 的逐字复刻：先打 `[BASELINE] … 存量已基线豁免（ratchet，只拦新增）`（仅当有存量），
 * 再按 `report` 格式打**新增**（fresh）。判定分族走 `./baseline.partition`（**判定单源不变**，本助手只管输出）。
 */
export function legacyGateKitRatchet(
  rule: string,
  violations: readonly Violation[],
  keys: ReadonlySet<string>,
  keyOf: (v: Violation) => string,
  hints: readonly string[],
  io: CheckIo,
  fail: boolean,
): boolean {
  const { fresh, grandfathered } = baseline.partition(violations, keys, keyOf);
  if (grandfathered.length > 0) {
    io.out(
      `[BASELINE] ${rule}: ${grandfathered.length} 处存量已基线豁免（ratchet，只拦新增）\n`,
    );
  }
  return legacyGateKitReport(rule, fresh, hints, io, fail);
}

export function defineCheck(decl: CheckDeclaration): RegisteredCheck {
  if (typeof decl.selfTest !== "function") {
    throw new Error(
      `${decl.id}: defineCheck 要求 selfTest（R1 判据自证）——缺自测者 MUST NOT 注册`,
    );
  }
  {
    const vf = decl.valueFlags ?? [];
    if (new Set(vf).size !== vf.length)
      throw new Error(`${decl.id}: valueFlags 有重复项`);
    const fset = new Set(decl.flags ?? []);
    for (const v of vf) {
      if (!v.startsWith("--"))
        throw new Error(`${decl.id}: 取值旗标 MUST 以 -- 开头（${v}）`);
      if (fset.has(v))
        throw new Error(
          `${decl.id}: 旗标 ${v} 同时声明为布尔与取值旗标（二者互斥）`,
        );
      if ((BASE_FLAGS as readonly string[]).includes(v)) {
        throw new Error(`${decl.id}: 取值旗标不得复用基板旗标 ${v}`);
      }
    }
  }
  if (decl.channels !== undefined) {
    if (
      decl.baseline !== undefined ||
      decl.label !== undefined ||
      decl.hints !== undefined ||
      decl.enforcement !== undefined ||
      decl.report !== undefined
    ) {
      throw new Error(
        `${decl.id}: channels 与顶层 label/hints/baseline/enforcement/report 互斥（通道各自的报告面 MUST 只在通道内声明）`,
      );
    }
    if (decl.channels.length === 0)
      throw new Error(
        `${decl.id}: channels 为空数组（要么省略，要么至少一条）`,
      );
  }

  return {
    decl,
    async main(argv = process.argv.slice(2)): Promise<number> {
      // 退出码 = 判定结果（2026-10-08 修复「迁移后检查静默假绿」）：检查模板
      // `if (import.meta.main) check.main();` 不消费返回值，而 bun 对未 await 的 promise 恒以 0 退出
      // ⇒ 有违规也 rc=0。基座内统一设 `process.exitCode`（单一实现），模板无需改动。
      const done = (code: number): number => {
        process.exitCode = code;
        return code;
      };
      const root = substrateRoot();
      const parsed = parseCheckArgs(decl, argv); // 同步抛：未声明旗标 / 检查层 --only / 缺取值
      if (parsed.selfTest) {
        const code = await decl.selfTest();
        return done(typeof code === "number" ? code : 0);
      }
      // 域语义由声明决定（保形）：`staged` = 无视模式恒取暂存集（旧 `stagedFiles()` 类检查）
      const effective: ParsedCheckArgs =
        decl.domain === "staged" && !parsed.staged && parsed.diff === null
          ? { ...parsed, staged: true }
          : parsed;
      const ctx = buildContext(root, effective);
      const violations = await decl.predicate(ctx);

      // 私有旗标钩子（在报告/棘轮之前）：返回 number = 以该码退出
      for (const flag of parsed.flags) {
        const code = decl.onFlag?.(flag, ctx, violations);
        if (typeof code === "number") return done(code);
      }

      if (parsed.updateBaseline) {
        if (decl.baseline === undefined)
          throw new Error(
            `${decl.id}: 未声明 baseline，不接受 --update-baseline`,
          );
        // 基线 = 仓库内容快照 ⇒ MUST NOT 收录未跟踪路径的键（否则其键会「预先豁免」日后入库的真违规）
        const trackedOnly = violations.filter((v) =>
          isTrackedPath(root, v.path.replace(/:\d+$/, "")),
        );
        const content =
          "# 基线（ratchet，只减不增）：每行一键，`#` 起始为注释\n" +
          `# rule_reference: ${decl.baseline.ruleReference}\n` +
          `${[...new Set(trackedOnly.map((v) => decl.baseline!.keyOf(v)))].sort().join("\n")}\n`;
        baselineStore.writeBaseline(root, decl.baseline.file, content);
        process.stderr.write(
          `${decl.id}: 已写基线 ${decl.baseline.file}（${trackedOnly.length} 条；IO 归 baseline-store）\n`,
        );
      }

      // 报告：单通道（顶层字段）或多通道（`channels`，按声明顺序逐通道）
      const channelDefs: readonly CheckChannel[] = decl.channels ?? [
        {
          id: decl.id,
          label: decl.label ?? decl.id,
          hints: decl.hints,
          baseline: decl.baseline,
          enforcement: decl.enforcement,
        },
      ];
      // ratchet 口径：基线 = 「仓库内容」的快照 ⇒ 未入库的在途文件（会话草稿 / 负例夹具）不属于仓库，
      // MUST NOT 计作新增违规（否则任何会话的工作树草稿都会阻断全仓推送）。**仅**对声明了基线的通道生效；
      // 非 ratchet 检查器（尤其安全 / 密钥类）保持扫描整树 —— 未跟踪的散落风险件照拦。
      const groups = channelDefs.map((ch) => {
        const mine = violations.filter((v) =>
          decl.channels === undefined ? true : v.channel === ch.id,
        );
        return {
          ch,
          own:
            ch.baseline === undefined
              ? mine
              : mine.filter((v) =>
                  isTrackedPath(root, v.path.replace(/:\d+$/, "")),
                ),
        };
      });
      const split = groups.map((g) => ({
        ...g,
        keys:
          g.ch.baseline === undefined
            ? null
            : baseline.loadKeys(root, g.ch.baseline.file),
      }));
      const freshOf = (g: (typeof split)[number]): Violation[] =>
        g.keys === null
          ? g.own
          : baseline.partition(g.own, g.keys, g.ch.baseline!.keyOf).fresh;

      if (decl.report !== undefined) {
        // **自定义报告面**（迁移缝）：检查自绘 stdout；返回 number 即以该码退出，否则按常规判定（但不再打印基板格式）
        const g = split[0]!;
        const fresh = freshOf(g);
        const code = decl.report(
          ctx,
          {
            violations: g.own,
            fresh,
            grandfathered: g.own.length - fresh.length,
            fail: parsed.fail,
            json: parsed.json,
          },
          stdIo(),
        );
        if (typeof code === "number") return done(code);
        return done(
          blockedBy(fresh.length, parsed.fail, g.ch.enforcement) ? 1 : 0,
        );
      }

      if (parsed.json) {
        process.stdout.write(
          `${JSON.stringify(
            decl.channels === undefined
              ? {
                  id: decl.id,
                  violations,
                  fresh: freshOf(split[0]!),
                  grandfathered:
                    split[0]!.own.length - freshOf(split[0]!).length,
                }
              : {
                  id: decl.id,
                  channels: split.map((g) => ({
                    id: g.ch.id,
                    violations: g.own,
                    fresh: freshOf(g),
                  })),
                },
            null,
            2,
          )}\n`,
        );
      } else {
        for (const g of split) {
          const fresh = freshOf(g);
          const grandfathered = g.own.length - fresh.length;
          if (grandfathered > 0) {
            process.stdout.write(
              `[BASELINE] ${g.ch.label}: ${grandfathered} 处存量已基线豁免（ratchet，只拦新增）\n`,
            );
          }
          if (fresh.length === 0) {
            process.stdout.write(`[OK] ${g.ch.label}: 无违规\n`);
          } else {
            process.stdout.write(
              `[VIOLATION] ${g.ch.label}: ${fresh.length} 处\n`,
            );
            for (const v of fresh.slice(0, 50)) {
              process.stdout.write(
                `  ${v.path}  ${(v.detail ?? v.message).slice(0, 120)}\n`,
              );
            }
            if (fresh.length > 50)
              process.stdout.write(`  … 另 ${fresh.length - 50} 处\n`);
            for (const h of g.ch.hints ?? [])
              process.stdout.write(`修复：${h}\n`);
          }
        }
      }

      const blocked = split.some((g) =>
        blockedBy(freshOf(g).length, parsed.fail, g.ch.enforcement),
      );
      return done(blocked ? 1 : 0);
    },
  };
}

/** 基板自证（含负例；`--self-test`）。 */
export async function substrateSelfTest(): Promise<number> {
  const probe: Pick<CheckDeclaration, "id" | "scopeValue"> = {
    id: "probe",
    scopeValue: "path",
  };
  const cases: Array<{ name: string; ok: boolean; detail: string }> = [];

  cases.push({
    name: "负例：缺 selfTest ⇒ 注册即抛",
    ok: (() => {
      try {
        defineCheck({
          id: "no-selftest",
          predicate: () => [],
        } as unknown as CheckDeclaration);
        return false;
      } catch (e) {
        return String(e).includes("selfTest");
      }
    })(),
    detail: "R1",
  });

  cases.push({
    name: "正例：三段式注册成功",
    ok:
      defineCheck({ id: "positive", predicate: () => [], selfTest: () => 0 })
        .decl.id === "positive",
    detail: "registered",
  });

  cases.push({
    name: "负例：检查层 --only ⇒ 抛错并提示 --scope",
    ok: (() => {
      try {
        parseCheckArgs(probe, ["--only", "anything"]);
        return false;
      } catch (e) {
        return String(e).includes("--only") && String(e).includes("--scope");
      }
    })(),
    detail: "only-rejected",
  });

  cases.push({
    name: "负例：未声明 scopeValue 时 --scope ⇒ 抛错",
    ok: (() => {
      try {
        parseCheckArgs({ id: "probe", scopeValue: undefined }, [
          "--scope",
          "x",
        ]);
        return false;
      } catch (e) {
        return String(e).includes("scopeValue");
      }
    })(),
    detail: "scope-undeclared",
  });

  cases.push({
    name: "负例：未声明旗标 ⇒ 抛错（不得静默忽略）",
    ok: (() => {
      try {
        parseCheckArgs(probe, ["--frobnicate"]);
        return false;
      } catch (e) {
        return String(e).includes("未声明");
      }
    })(),
    detail: "unknown-flag",
  });

  cases.push({
    name: "语义收敛：--cached ⇒ staged 且蕴含 fail",
    ok: (() => {
      const a = parseCheckArgs(probe, ["--cached"]);
      return a.staged && a.fail;
    })(),
    detail: "cached-implies-fail",
  });

  cases.push({
    name: "已声明自有旗标 ⇒ 受理并暴露于 flags",
    ok: (() => {
      const a = parseCheckArgs({ ...probe, flags: ["--dump-baseline"] }, [
        "--dump-baseline",
      ]);
      return a.flags.has("--dump-baseline");
    })(),
    detail: "declared-flag-accepted",
  });

  cases.push({
    name: "负例：未声明的自有旗标 ⇒ 抛错（接口不得静默忽略）",
    ok: (() => {
      try {
        parseCheckArgs(probe, ["--dump-baseline"]);
        return false;
      } catch (e) {
        return String(e).includes("未声明");
      }
    })(),
    detail: "undeclared-flag-rejected",
  });

  cases.push({
    name: "夹具上下文：fixtureContext 的 walk 按扩展名过滤（防「非目标扩展名被扫」假例）",
    ok: (() => {
      const ctx = fixtureContext({ "a.ts": "TODO", "b.rs": "TODO" });
      return ctx.walk(["ts"]).length === 1 && ctx.walk(["ts"])[0] === "a.ts";
    })(),
    detail: "fixture-walk-fidelity",
  });

  cases.push({
    name: "负例：channels 与顶层 label/baseline 混用 ⇒ 注册即抛",
    ok: (() => {
      try {
        defineCheck({
          id: "bad-channels",
          label: "顶层 label",
          channels: [{ id: "c1", label: "通道一" }],
          predicate: () => [],
          selfTest: () => 0,
        } as unknown as CheckDeclaration);
        return false;
      } catch (e) {
        return String(e).includes("互斥");
      }
    })(),
    detail: "channels-exclusive",
  });

  cases.push({
    name: "正例：stdIo() 暴露 out/err 双流（report 缝第三参的 stderr 承载面；补 `\\n` 即 console.* 等价）",
    ok: (() => {
      const io = stdIo();
      return typeof io.out === "function" && typeof io.err === "function";
    })(),
    detail: "io-dual-stream",
  });

  cases.push({
    name: "负例：channels 与 report 缝混用 ⇒ 注册即抛（报告面归通道）",
    ok: (() => {
      try {
        defineCheck({
          id: "bad-report-channels",
          channels: [{ id: "c1", label: "通道一" }],
          report: () => 0,
          predicate: () => [],
          selfTest: () => 0,
        } as unknown as CheckDeclaration);
        return false;
      } catch (e) {
        return String(e).includes("互斥") && String(e).includes("report");
      }
    })(),
    detail: "report-exclusive",
  });

  cases.push({
    name: "阻断判定唯一实现：on-fail 只在 --fail 下阻断；always 无视 --fail；0 违规恒不阻断",
    ok:
      blockedBy(1, false, undefined) === false &&
      blockedBy(1, true, undefined) === true &&
      blockedBy(1, false, "always") === true &&
      blockedBy(0, true, "always") === false,
    detail: "blocked-single-source",
  });

  cases.push({
    name: "正例：channels 声明（各通道独立 label）注册成功且不抛",
    ok:
      defineCheck({
        id: "ok-channels",
        channels: [
          { id: "hard", label: "硬阻断道", enforcement: "always" },
          {
            id: "ratchet",
            label: "棘轮道",
            baseline: {
              file: "x.txt",
              ruleReference: "y",
              keyOf: (v) => v.path,
            },
          },
        ],
        predicate: () => [],
        selfTest: () => 0,
      }).decl.id === "ok-channels",
    detail: "channels-ok",
  });

  const ctx = buildContext(substrateRoot(), {
    fail: false,
    json: false,
    updateBaseline: false,
    selfTest: false,
    staged: false,
    diff: null,
    scopeValue: null,
    positional: [],
    flags: new Set<string>(),
  });
  cases.push({
    name: "walk 与 walkDirs 可用且互不含混（walk 只文件 / walkDirs 只目录）",
    ok: (() => {
      const files = ctx.walk(["ts"]);
      const dirs = ctx.walkDirs();
      return (
        files.length > 0 &&
        dirs.length > 0 &&
        files.every((f) => !dirs.includes(f)) &&
        files.every((f) => f.endsWith(".ts"))
      );
    })(),
    detail: "traversal",
  });
  cases.push({
    name: "文件域无输入 ⇒ 空数组（不误判为「无违规」而是「无域输入」）",
    ok: fileDomain(substrateRoot(), { staged: false, diff: null }).length === 0,
    detail: "empty-domain",
  });

  // 旧 gate-kit 报告格式复刻（迁移缝）：正例断言字节 + 返回码语义；负例断言 `:行号` 不被二次补位
  cases.push({
    name: "legacyGateKitReport：无违规 ⇒ `[OK] rule: 无违规` 且不阻断；有违规 ⇒ 逐字一行 + `修复：` 且返回 fail",
    ok: (() => {
      const grab = (): { io: CheckIo; out: string[] } => {
        const out: string[] = [];
        return {
          io: {
            out: (s: string) => out.push(s),
            err: (s: string) => out.push(`ERR${s}`),
          },
          out,
        };
      };
      const a = grab();
      const okA =
        legacyGateKitReport("R", [], ["h1"], a.io, true) === false &&
        a.out.join("") === "[OK] R: 无违规\n";
      const b = grab();
      const vs: Violation[] = [
        { path: "f/a.ts", message: "m1" },
        { path: "f/b.ts:42", message: "m2" },
      ];
      const blocked = legacyGateKitReport("R", vs, ["h"], b.io, true);
      const bytes = b.out.join("");
      const want =
        "[VIOLATION] R: 2 处\n  f/a.ts:1  m1\n  f/b.ts:42  m2\n修复：h\n";
      const okB = blocked === true && bytes === want;
      const c = grab();
      const okC = legacyGateKitReport("R", vs, ["h"], c.io, false) === false;
      return okA && okB && okC;
    })(),
    detail: "legacy-report-bytes",
  });

  cases.push({
    name: "legacyGateKitRatchet：有存量 ⇒ 先打 `[BASELINE]` 行，再只打新增（keyOf 单源分区）",
    ok: (() => {
      const out: string[] = [];
      const io: CheckIo = {
        out: (s: string) => out.push(s),
        err: (s: string) => out.push(`ERR${s}`),
      };
      const vs: Violation[] = [
        { path: "old.ts", message: "存量" },
        { path: "new.ts", message: "新增" },
      ];
      const blocked = legacyGateKitRatchet(
        "R",
        vs,
        new Set(["old.ts"]),
        (v) => v.path,
        ["hint"],
        io,
        true,
      );
      const want =
        "[BASELINE] R: 1 处存量已基线豁免（ratchet，只拦新增）\n[VIOLATION] R: 1 处\n  new.ts:1  新增\n修复：hint\n";
      return blocked === true && out.join("") === want;
    })(),
    detail: "legacy-ratchet-bytes",
  });

  // 异步 selfTest：判定核为 async 的检查（如 code-inline-sql 的 judgeInlineCode）需要它；
  // 断言**实跑**（main(['--self-test']) 的返回码 = 该 async 返回码），而非仅类型面
  cases.push({
    name: "async selfTest 被 await 并以其返回码退出（rc=7）",
    ok:
      (await defineCheck({
        id: "async-selftest",
        predicate: () => [],
        selfTest: async () => 7,
      }).main(["--self-test"])) === 7,
    detail: "async-selftest",
  });

  cases.push({
    name: "取值旗标：可重复并按出现顺序收集取值，且不落 argv（positional）",
    ok: (() => {
      const p = parseCheckArgs(
        {
          id: "vf",
          scopeValue: undefined,
          flags: [],
          valueFlags: ["--file"],
        } as unknown as Pick<
          CheckDeclaration,
          "id" | "scopeValue" | "flags" | "valueFlags"
        >,
        ["--file", "a.txt", "--file", "b.txt", "c.txt"],
      );
      const vals = p.valueFlags.get("--file") ?? [];
      return (
        vals.length === 2 &&
        vals[0] === "a.txt" &&
        vals[1] === "b.txt" &&
        p.positional.length === 1 &&
        p.positional[0] === "c.txt"
      );
    })(),
    detail: "value-flags-repeatable",
  });
  cases.push({
    name: "负例：取值旗标缺取值 ⇒ 解析抛错（fail-closed）",
    ok: (() => {
      try {
        parseCheckArgs(
          {
            id: "vf2",
            scopeValue: undefined,
            flags: [],
            valueFlags: ["--file"],
          } as unknown as Pick<
            CheckDeclaration,
            "id" | "scopeValue" | "flags" | "valueFlags"
          >,
          ["--file"],
        );
        return false;
      } catch (e) {
        return String(e).includes("缺取值");
      }
    })(),
    detail: "value-flags-missing-value",
  });

  let failed = 0;
  for (const c of cases) {
    process.stdout.write(
      `${c.ok ? "PASS" : "FAIL"}  ${c.name}（${c.detail}）\n`,
    );
    if (!c.ok) failed += 1;
  }
  process.stdout.write(
    `\ncheck-interface self-test: ${cases.length - failed}/${cases.length} passed\n`,
  );
  return failed === 0 ? 0 : 1;
}

if (import.meta.main && process.argv.includes("--self-test"))
  process.exit(await substrateSelfTest());
