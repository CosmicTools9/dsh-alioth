#!/usr/bin/env bun
/**
 * baseline-store.ts — 基线数据（ratchet 基线 / 存量登记）的**唯一读写入口**。
 *
 * 背景（2026-09-24 namespace 拆分）：门禁基线是**共享可变状态**。ns 仓经 `scripts` 符号链接
 * 直接读写平台仓的 `scripts/check/**`，于是「在某个 ns 仓执行 `--write-baseline`」会整文件覆盖
 * 平台单源基线，抹掉其他 ns 的存量豁免条目，而该改动在 ns 仓 `git status` 中不可见
 * （符号链接不被 git 跟踪）⇒ 成为留在平台工作树的「幽灵改动」。
 *
 * 本模块用两条不变式把上述可能从结构上消弭（不是加提示，是让代码路径不可能）：
 *
 *   INV-1【写入方唯一】基线的物理写入目标 MUST 落在**本仓工作树内**（物理路径 realpath 判定）。
 *        ns 仓的 `scripts/` 是指向平台仓的符号链接 ⇒ 任何以 `scripts/check/...` 为目标的写入
 *        其 realpath 落在本仓之外，一律抛 `BaselineOwnershipError` 并中止。
 *        ⇒ 分裂漂移的必要条件（存在第二个写入方）在运行期被消除。
 *
 *   INV-2【读取按归属过滤】读取时逐行判定：若该行出现 `Pre-Proc/<ns>/` 且全部出现处都不是
 *        本仓 ns ⇒ 丢弃该行。无 `Pre-Proc/` 引用的行（平台内容条目、DB 列名类键、头部注释）
 *        一律保留。平台仓不做过滤（全量视图）。
 *        ⇒ ns 仓不再看见其他 ns 的死条目（判定结果不变：那些条目在本地永不命中）。
 *
 * 归属判定沿用 `scripts/lib/ns-surface.ts` 的口径：以「本仓是否跟踪 `Pre-Proc/<ns>/Sources/…`」
 * 为依据（而非磁盘目录是否存在），避免开发机符号链接导致判定翻转。见 `.planning/tool-drift-analysis.md`。
 *
 * CLI（供 shell 侧调用，避免 shell 直接重定向写共享面）：
 *   bun scripts/check/lib/baseline-store.ts owner <root>
 *   bun scripts/check/lib/baseline-store.ts path  <root> <canonicalRel>
 *   bun scripts/check/lib/baseline-store.ts read  <root> <canonicalRel>
 *   bun scripts/check/lib/baseline-store.ts write <root> <canonicalRel>   # 内容走 stdin
 */
import { spawnSync } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
  rmSync,
  statSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, join, resolve, sep } from "node:path";
import { repoRootOf } from "../../lib/repo-role";

/** 归属冲突（跨仓写 / 越界写）——恒为硬失败，不做降级。 */
export class BaselineOwnershipError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "BaselineOwnershipError";
  }
}

const NS_REF = /Pre-Proc\/([A-Za-z0-9_-]+)\//g;

/** 归属判定缓存（同一进程内 git 调用只做一次）。 */
const ownerCache = new Map<string, string | null>();

/**
 * 仓库根（起始点可以是目录、文件、或尚不存在的路径）。
 *
 * **写入方身份由此起点决定**（`start` 通常取进程 cwd 或脚本自身位置）——MUST NOT 由「目标路径」
 * 反推，否则守卫会与目标自洽而形同虚设（示例：ns 仓内以平台目标路径反推 root ⇒ 二者相等 ⇒ 放行）。
 */
export function repoRoot(start = "."): string {
  let dir = physicalPath(start);
  for (;;) {
    if (existsSync(dir) && statSync(dir).isDirectory()) break;
    const up = dirname(dir);
    if (up === dir) break;
    dir = up;
  }
  const out = repoRootOf(dir);
  return out !== null ? realpathSync(out) : dir;
}

/**
 * 本仓归属的 namespace。
 *
 * - 返回 `null` ⇒ **平台仓**（不跟踪任何 ns 源；基线数据的唯一写入方）。
 * - 返回 ns 名 ⇒ **namespace 仓**（跟踪该 ns 的 `Pre-Proc/<ns>/Sources/…`；基线只读）。
 */
export function ownerNs(root: string = "."): string | null {
  const key = resolve(root);
  const hit = ownerCache.get(key);
  if (hit !== undefined) return hit;
  const r = spawnSync(
    "git",
    ["-C", root, "ls-files", "-z", "Pre-Proc/*/Sources/*"],
    {
      encoding: "utf8",
    },
  );
  const names = new Set<string>();
  for (const f of (r.stdout ?? "").split("\u0000")) {
    if (f === "") continue;
    const m = /^Pre-Proc\/([^/]+)\/Sources\//.exec(f);
    if (m) names.add(m[1]);
  }
  const value = names.size === 1 ? [...names][0] : null;
  ownerCache.set(key, value);
  return value;
}

/**
 * 路径的**物理**形态（解析沿途每一段符号链接，含尚不存在的尾段）。
 *
 * `realpathSync` 要求路径存在，故自叶向上找到第一个存在的祖先，再拼回尾段。
 */
export function physicalPath(p: string): string {
  let cur = resolve(p);
  const tail: string[] = [];
  for (;;) {
    if (existsSync(cur)) return join(realpathSync(cur), ...tail.reverse());
    const parent = dirname(cur);
    if (parent === cur) return resolve(p);
    tail.push(basename(cur));
    cur = parent;
  }
}

/** 物理路径是否位于 `root` 工作树之内。 */
export function isInside(root: string, target: string): boolean {
  const r = physicalPath(root);
  const t = physicalPath(target);
  return t === r || t.startsWith(r + sep);
}

/**
 * 目标物理路径所属的 git 仓根（不含任何 git 仓时返回 `null`）。
 *
 * 用途：区分三类写入目标——
 *   ① 本仓工作树内        ⇒ 合法（平台仓写自己的 `scripts/check/**`）；
 *   ② **另一个** git 仓内  ⇒ 越界（ns 仓经符号链接写进平台仓 checkout = 漂移源）；
 *   ③ 不属于任何 git 仓    ⇒ 临时目录 / 测试夹具 ⇒ 合法。
 */
export function enclosingRepo(target: string): string | null {
  let dir = physicalPath(target);
  // `git -C` 只接受目录：自目标向上找到第一个存在的**目录**（目标本身可能是文件）。
  for (;;) {
    if (existsSync(dir) && statSync(dir).isDirectory()) break;
    const up = dirname(dir);
    if (up === dir) return null;
    dir = up;
  }
  const out = repoRootOf(dir);
  return out === null ? null : realpathSync(out);
}

/** 规范化目标：相对路径按仓库根解析；绝对路径原样。 */
export function canonicalTarget(root: string, rel: string): string {
  return rel.startsWith("/") ? resolve(rel) : join(root, rel);
}

/**
 * INV-1 守卫：断言该目标可写，返回其物理路径。
 *
 * 拒绝的两种情况（均为硬失败）：
 *   ① 目标物理路径落在**另一个** git 仓的工作树内 —— 典型即 ns 仓经 `scripts` 符号链接写进
 *      平台仓 checkout（共享面写入 = 覆盖其他 ns 的存量登记，且形成幽灵改动）；
 *      「不属于任何 git 仓」的路径（临时目录 / 测试夹具）照常放行。
 *   ② 本仓归属某 ns 时，写入内容含指向**其他 ns** 的条目（防混合污染）。
 */
export function assertWriteAllowed(
  root: string,
  target: string,
  content?: string,
): string {
  const physical = physicalPath(target);
  const home = physicalPath(root);
  const enclosing = enclosingRepo(target);
  if (enclosing !== null && enclosing !== home) {
    const ns = ownerNs(root);
    const who =
      ns === null ? "另一个 git 仓" : `平台仓（ns 仓「${ns}」的共享面）`;
    throw new BaselineOwnershipError(
      [
        `[BASELINE-OWNERSHIP] 拒绝写基线：目标物理路径落在${who}的工作树内。`,
        `  目标: ${target}`,
        `  物理: ${physical}`,
        `  所属仓: ${enclosing}`,
        `  规则: 基线数据的写入方唯一（INV-1）——共享面 scripts/check/** 只允许在平台仓内写入。`,
        ns === null
          ? "  ⇒ 本仓即平台仓：请检查目标是否误指向其他 checkout。"
          : `  ⇒ ns 仓「${ns}」不得重生成共享基线（否则覆盖其他 ns 的存量登记，且改动在 ns 仓 git status 中不可见）。`,
        ns === null
          ? ""
          : "  ⇒ 需重生成 B 类基线时，请在**平台仓**运行重生成入口：`bun scripts/check/baselines/regen-ns-face.ts --list`（就绪态），`--tool <名称> [--ns <NS>] [--check]`（`--check` 预演；内容取自 ns 仓、写入平台单源）。",
      ]
        .filter((l) => l !== "")
        .join("\n"),
    );
  }
  const ns = ownerNs(root);
  if (ns !== null && content !== undefined) {
    const foreign = foreignNsRefs(content, ns);
    if (foreign.size > 0) {
      throw new BaselineOwnershipError(
        [
          `[BASELINE-OWNERSHIP] 拒绝写基线：本仓归属 ns「${ns}」，但内容含其他 ns 的条目。`,
          `  目标: ${target}`,
          `  越界 ns: ${[...foreign].sort().join(", ")}`,
          "  规则: 每个 ns 的存量登记只登记本 ns 内容（INV-1b）；跨 ns 条目属对应 ns 仓/平台仓职责。",
        ].join("\n"),
      );
    }
  }
  return physical;
}

/**
 * INV-2：内容中出现的、且不等于 `own` 的 ns 集合。
 *
 * 仅统计 `Pre-Proc/<ns>/` 形态的显式引用；DB 列名、平台相对路径等无该前缀者不计。
 */
export function foreignNsRefs(content: string, own: string): Set<string> {
  const found = new Set<string>();
  for (const line of content.split("\n")) {
    for (const m of line.matchAll(NS_REF)) if (m[1] !== own) found.add(m[1]);
  }
  return found;
}

/**
 * INV-2 过滤：丢弃「只指向其他 ns」的行，保留本 ns 行与无 ns 引用的行。
 *
 * 对 JSON 基线同样安全：`JSON.stringify(x, null, 2)` 保证每个数组元素独占一行，
 * 且元素内不含裸换行（转义为 `\n`）⇒ 行即条目。
 */
export function filterForOwner(content: string, own: string | null): string {
  if (own === null) return content;
  const keep = (s: string): boolean => {
    const refs = [...s.matchAll(NS_REF)].map((m) => m[1]);
    return refs.length === 0 || refs.some((n) => n === own);
  };
  // JSON 基线（多 ns 单源）：逐**元素**过滤并重新序列化——按行过滤会破坏 JSON 结构
  // （丢行 ⇒ 逗号/括号失衡）⇒ 调用方 `JSON.parse` 失败 ⇒ 基线被误判为空
  // ⇒ ns 仓对任何违规都判「新增」（2026-09-26 block-json 在 Alioth 仓实证）。
  try {
    const doc: unknown = JSON.parse(content);
    if (doc !== null && typeof doc === "object" && !Array.isArray(doc)) {
      const rec = doc as Record<string, unknown>;
      let touched = false;
      for (const [k, v] of Object.entries(rec)) {
        if (!Array.isArray(v)) continue;
        const kept = v.filter((item) =>
          keep(typeof item === "string" ? item : JSON.stringify(item)),
        );
        if (kept.length !== v.length) {
          rec[k] = kept;
          touched = true;
        }
      }
      return touched ? `${JSON.stringify(doc, null, 2)}\n` : content;
    }
  } catch {
    // 非 JSON ⇒ 行过滤（TSV/纯文本基线）
  }
  return content.split("\n").filter(keep).join("\n");
}

/** 解析基线写入目标（供脚本构造常量；真正的越界拦截在 `writeBaseline`）。 */
export function baselineTargetPath(root: string, rel: string): string {
  return canonicalTarget(root, rel);
}

/**
 * 唯一基线写入入口。
 *
 * @param root 仓库根
 * @param rel  基线路径（相对仓库根，或绝对路径）
 */
export function writeBaseline(
  root: string,
  rel: string,
  content: string,
): void {
  const target = canonicalTarget(root, rel);
  assertWriteAllowed(root, target, content);
  mkdirSync(dirname(target), { recursive: true });
  writeFileSync(target, content);
}

/**
 * 唯一基线读取入口（文本形态）。
 *
 * @returns 过滤后的内容；文件不存在时返回 `null`（调用方按空基线处理 = fail-closed）。
 */
export function readBaselineText(root: string, rel: string): string | null {
  const target = canonicalTarget(root, rel);
  if (!existsSync(target)) return null;
  if (statSync(target).isDirectory()) return null;
  return filterForOwner(readFileSync(target, "utf8"), ownerNs(root));
}

/** 读取并按行切出**数据行**（去空行、去 `#` 注释）。 */
export function readBaselineLines(root: string, rel: string): string[] {
  const text = readBaselineText(root, rel);
  if (text === null) return [];
  return text
    .split("\n")
    .map((l) => l.trimEnd())
    .filter((l) => l.trim() !== "" && !l.trimStart().startsWith("#"));
}

/**
 * 基线**行语法归一化**（change `deepen-check-substrate` tasks 3.3；单一行语法）。
 *
 * 唯一语法（无损，对全部读取器等价）：
 *   · 注释行（`ltrimStart()` 后以 `#` 开头）与原数据行**原样保留**，只做 `trimEnd()`；
 *   · 数据行做首尾 `trim()`（**不动行内分隔符**——`\t` 列语义载体的字段数不变）；
 *   · 丢弃空行；丢弃**与已见数据行完全相同**的重复行（保序）。
 *
 * 为何无损：`readBaselineLines` 本就 `trimEnd()` + 跳空行 + 跳注释；`readBaselineKeys` 取首段 ⇒
 * 归一化后 `keys(归一化(x)) === keys(x)` 且多列载体字段切分结果不变（自测逐例断言）。
 */
export function normalizeBaselineText(text: string): string {
  const out: string[] = [];
  const seen = new Set<string>();
  for (const raw of text.split("\n")) {
    const line = raw.trimEnd();
    if (line.trim() === "") continue;
    if (line.trimStart().startsWith("#")) {
      out.push(line);
      continue;
    }
    const data = line.trim();
    if (seen.has(data)) continue;
    seen.add(data);
    out.push(data);
  }
  return out.length > 0 ? `${out.join("\n")}\n` : "";
}

/** 读取并取每行首段为键（与 `gate-kit.loadBaseline` 同口径）。 */
export function readBaselineKeys(root: string, rel: string): Set<string> {
  return new Set(
    readBaselineLines(root, rel)
      .map((l) => l.trim().split(/\s+/)[0])
      .filter((k) => k.length > 0),
  );
}

// ── CLI ────────────────────────────────────────────────────────────────────

function usage(): never {
  console.error(
    "用法: baseline-store.ts <owner|path|read|write> <root> [canonicalRel]\n" +
      "  owner <root>                     打印本仓归属（platform | <ns>）\n" +
      "  path  <root> <rel>               打印基线写入目标（不校验）\n" +
      "  read  <root> <rel>               打印过滤后的基线内容\n" +
      "  write <root> <rel>               从 stdin 读内容并写入（带 INV-1 守卫）",
  );
  process.exit(2);
}

function main(): number {
  const [cmd, root, rel] = process.argv.slice(2);
  if (!cmd || !root) usage();
  switch (cmd) {
    case "owner": {
      const ns = ownerNs(root);
      process.stdout.write(`${ns ?? "platform"}\n`);
      return 0;
    }
    case "path": {
      if (!rel) usage();
      process.stdout.write(`${baselineTargetPath(root, rel)}\n`);
      return 0;
    }
    case "read": {
      if (!rel) usage();
      const text = readBaselineText(root, rel);
      if (text !== null) process.stdout.write(text);
      return 0;
    }
    case "write": {
      if (!rel) usage();
      const content = readFileSync(0, "utf8");
      writeBaseline(root, rel, content);
      return 0;
    }
    default:
      usage();
  }
}

// ── 自测 ───────────────────────────────────────────────────────────────────

function selfTest(): number {
  const failures: string[] = [];
  const check = (name: string, ok: boolean, detail = ""): void => {
    if (!ok) failures.push(`${name}${detail ? ` — ${detail}` : ""}`);
  };

  // INV-2：过滤只保留本 ns 行 + 无 ns 引用的行
  const mixed = [
    "# 头注释",
    "Pre-Proc/WZ/Sources/App.tsx",
    "Pre-Proc/AVIC-CAASEC/Sources/App.tsx",
    "Framework/backend/x.rs:kind",
    '{"file":"Pre-Proc/SE/Sources/y.tsx"}',
  ].join("\n");
  const filtered = filterForOwner(mixed, "WZ");
  check(
    "filter 保留本 ns 行",
    filtered.includes("Pre-Proc/WZ/Sources/App.tsx"),
  );
  check("filter 丢弃他 ns 行", !filtered.includes("AVIC-CAASEC"));
  check("filter 丢弃他 ns 行（JSON 形态）", !filtered.includes("SE/Sources"));
  check(
    "filter 保留无 ns 引用行",
    filtered.includes("Framework/backend/x.rs:kind"),
  );
  check("filter 保留注释行", filtered.includes("# 头注释"));
  check("平台档位不过滤", filterForOwner(mixed, null) === mixed);

  // INV-2（格式感知，2026-09-26 修复）：JSON 文档 MUST 逐元素过滤且**过滤后仍可解析**
  // （按行过滤会丢行破坏结构 ⇒ 调用方解析失败 ⇒ 基线被误判为空；实测阻断过 Alioth/AVIC 推送）
  const doc =
    JSON.stringify(
      {
        note: "x",
        violations: [
          "WZ|Pre-Proc/WZ/Sources/a.tsx|R1|d",
          "Alioth|Pre-Proc/Alioth/Sources/b.tsx|R1|d",
          "plain-no-ref",
        ],
      },
      null,
      2,
    ) + "\n";
  const docOut = filterForOwner(doc, "WZ");
  let docParsed: { violations?: string[] } | null = null;
  try {
    docParsed = JSON.parse(docOut) as { violations?: string[] };
  } catch {
    docParsed = null;
  }
  check("JSON 基线过滤后仍可解析", docParsed !== null);
  check(
    "JSON 基线保本 ns 条目",
    (docParsed?.violations ?? []).some((v) => v.includes("Pre-Proc/WZ/")),
  );
  check(
    "JSON 基线丢他 ns 条目",
    !(docParsed?.violations ?? []).some((v) => v.includes("Alioth")),
  );
  check(
    "JSON 基线保无引用条目",
    (docParsed?.violations ?? []).includes("plain-no-ref"),
  );
  check("平台档位 JSON 原样", filterForOwner(doc, null) === doc);
  const objDoc = JSON.stringify(
    {
      sites: [
        { file: "Pre-Proc/WZ/a.rs", line: 1 },
        { file: "Pre-Proc/SE/b.rs", line: 2 },
      ],
    },
    null,
    2,
  );
  const objOut = JSON.parse(filterForOwner(objDoc, "WZ")) as {
    sites: Array<{ file: string }>;
  };
  check(
    "JSON 对象条目保本 ns",
    objOut.sites.length === 1 && objOut.sites[0].file.includes("Pre-Proc/WZ/"),
  );

  // INV-1b：越界 ns 引用识别
  check(
    "foreignNsRefs 只报他 ns",
    [...foreignNsRefs(mixed, "WZ")].sort().join(",") === "AVIC-CAASEC,SE",
  );

  // INV-1：物理围栏（只拦「写进另一个 git 仓」，放行临时目录/测试夹具）
  const here = repoRoot(".");
  check(
    "isInside 自身",
    isInside(here, join(here, "scripts/check/lib/baseline-store.ts")),
  );
  check(
    "isInside 拒绝外部路径",
    !isInside(here, "/tmp/alioth-baseline-probe.txt"),
  );
  check(
    "enclosingRepo 认出本仓（传入文件路径）",
    enclosingRepo(join(here, "scripts/check/lib/baseline-store.ts")) === here,
  );
  check(
    "enclosingRepo 对非仓路径返回 null",
    enclosingRepo("/tmp/alioth-baseline-probe/child.txt") === null,
  );

  // INV-1 回归（2026-09-25 事故）：经符号链接写「另一个 git 仓」必须被拒绝，且不得落盘。
  // 该场景就是 ns 仓写共享面的形状；先前 enclosingRepo 误传文件路径给 `git -C` ⇒ 判定为「无仓」⇒ 放行。
  const outer = mkdtempSync(join(tmpdir(), "baseline-store-selftest-"));
  try {
    const foreign = join(outer, "ForeignRepo");
    mkdirSync(join(foreign, "scripts", "check"), { recursive: true });
    spawnSync("git", ["-C", foreign, "init", "-q"]);
    const probe = join(foreign, "scripts", "check", "probe.txt");
    writeFileSync(probe, "orig\n");

    const local = join(outer, "LocalNsRepo");
    mkdirSync(join(local, "Pre-Proc", "WZ", "Sources", "Apps"), {
      recursive: true,
    });
    writeFileSync(
      join(local, "Pre-Proc", "WZ", "Sources", "Apps", "x.ts"),
      "// ns source\n",
    );
    spawnSync("git", ["-C", local, "init", "-q"]);
    spawnSync("git", ["-C", local, "add", "-A"]);
    symlinkSync(foreign, join(local, "scripts"), "dir");

    check("归属判定：符号链接仓认出 ns", ownerNs(local) === "WZ");
    let threw = false;
    try {
      writeBaseline(local, "scripts/check/probe.txt", "clobber\n");
    } catch (e) {
      threw = e instanceof BaselineOwnershipError;
    }
    check("INV-1 拒绝经符号链接写他仓", threw);
    check("INV-1 拒绝后未落盘", readFileSync(probe, "utf8") === "orig\n");

    // 拒绝「将其他 ns 条目写进本 ns 仓」的目标（INV-1b）
    let threwB = false;
    try {
      writeBaseline(
        local,
        join(local, "local-baseline.txt"),
        "Pre-Proc/AVIC-CAASEC/Sources/a.ts\n",
      );
    } catch (e) {
      threwB = e instanceof BaselineOwnershipError;
    }
    check("INV-1b 拒绝跨 ns 条目", threwB);

    // 非 git 仓路径（测试夹具）必须放行
    const fixture = join(outer, "fixture-baseline.txt");
    writeBaseline(local, fixture, "fixture entry\n");
    check(
      "放行非仓路径（测试夹具）",
      readFileSync(fixture, "utf8") === "fixture entry\n",
    );
  } finally {
    rmSync(outer, { recursive: true, force: true });
  }

  // 行语法归一化（tasks 3.3）：往返恒等——键集不变、注释保留、多列字段数不变、幂等
  const messy = [
    "# 头注释",
    "",
    "  a/b.ts:kind  ",
    "a/b.ts:kind",
    "\tc/d.rs\tR1\tdetail\t",
    "",
    "#note 保留",
  ].join("\n");
  const norm = normalizeBaselineText(messy);
  const keysOf = (t: string): string =>
    [
      ...new Set(
        t
          .split("\n")
          .filter((l) => l.trim() !== "" && !l.trimStart().startsWith("#"))
          .map((l) => l.trim().split(/\s+/)[0]),
      ),
    ].join(",");
  check(
    "归一化：键集不变（往返恒等）",
    keysOf(norm) === keysOf(messy),
    `${keysOf(norm)} vs ${keysOf(messy)}`,
  );
  check(
    "归一化：注释行保留",
    norm.includes("# 头注释") && norm.includes("#note 保留"),
  );
  check("归一化：空行清除", !norm.includes("\n\n"));
  check(
    "归一化：重复数据行去重",
    norm.split("\n").filter((l) => l.startsWith("a/b.ts:kind")).length === 1,
  );
  check(
    "归一化：多列载体字段数不变（\\t 列语义保留；仅首尾空白被裁——与 readBaselineLines 同口径）",
    (norm.split("\n").find((l) => l.startsWith("c/d.rs")) ?? "")
      .split("\t")
      .join("|") === "c/d.rs|R1|detail",
  );
  check("归一化：幂等", normalizeBaselineText(norm) === norm);

  if (failures.length > 0) {
    console.error("❌ baseline-store 自测失败：");
    for (const f of failures) console.error(`   - ${f}`);
    return 1;
  }
  console.log(
    "✅ baseline-store 自测通过（INV-1 物理围栏 + INV-1b 归属围栏 + INV-2 读侧过滤 + 行语法归一化往返）",
  );
  return 0;
}

if (import.meta.main) {
  const argv = process.argv.slice(2);
  process.exit(argv.includes("--self-test") ? selfTest() : main());
}
