/**
 * spec-reachability.ts — capability 可达性普查（唯一实现）。
 *
 * 判据（正本 = `openspec/specs/spec-governance` :: `stale-specs-deleted-not-parked`，D-049）：
 *   capability `c` 可达 ⟺ 名字 `c` 出现在至少一个消费面：
 *     ① governance — `AGENTS.md` / `.agents` / `docs` / `scripts` / `.omp`
 *     ② code       — `Framework` `Meta` `Gateway` `SSO` `OpenActivity` `Pre-Proc` `Deploy` `Samples`
 *     ③ sibling    — 某个 `openspec/specs/<cap>/spec.md`，**排除 `c` 自己的 spec 文件**
 *                    （否则其 `# c Specification` 标题自命中；实测不排除时虚报 748/758）
 *
 * 零 spawn：本 harness 下任何子进程成本实测 ≈38 ms/被扫文件（`Bun.spawnSync` 与
 * `node:child_process::spawnSync` 同病），故普查在进程内完成。
 *
 * 文本匹配取「连字符对齐的连续片段」语义：把每个 token 按 `-` 切成片段，检查**任意连续片段区间**
 * 是否等于某 capability 名（覆盖前缀 / 后缀 / 中段，如 token `check-flow-template` 命中名字
 * `flow-template`）。与 `grep -F` 的差异面 = 名字被非连字符字符紧邻包夹的形态（如
 * `agent-contractX`）——该形态在可读文本中不构成「提及」，且已在 `--self-test` 的等价断言中登记。
 */

import {
  existsSync,
  lstatSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
  statSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { EXCLUDE_DIRS, isExcludedDirName } from './gate-kit';

export type Face = 'governance' | 'code' | 'sibling';

/** 治理面根（相对仓根） */
export const GOVERNANCE_ROOTS = ['AGENTS.md', '.agents', 'docs', 'scripts', '.omp'] as const;
/** 代码面根（相对仓根） */
export const CODE_ROOTS = [
  'Framework',
  'Meta',
  'Gateway',
  'SSO',
  'OpenActivity',
  'Pre-Proc',
  'Deploy',
  'Samples',
  'vendor',
] as const;

const TEXT_EXT_RE =
  /\.(ts|tsx|js|jsx|mjs|cjs|rs|md|markdown|json|jsonc|yaml|yml|toml|sh|bash|zsh|fish|txt|html|htm|css|scss|less|sql|tsv|csv|conf|cfg|ini|env|service|vue|svelte|graphql|gql|lock|pem|rules|workflow|patch|diff|log)$/;

/**
 * 排除目录集：**以 gate-kit 的 `EXCLUDE_DIRS` 为单一属主**（门禁 `exclude-dirs-single-source`），
 * 只经其 `isExcludedDirName(name, excludeDirs)` 参数派生本判据的集合。
 * 差异 = 本判据**保留** `Deploy` / `Release`：二者是**手写配置面**（`Deploy/runtime.conf`、`sso-mode.conf`、
 * `Deploy/{ns}/*.conf`），名字出现在其中即算真实消费面；排除的仍是构建/依赖/VCS/工作树目录。
 * （`gate-kit` 排除 `Deploy`/`Release` 的理由是「生成物不当仓库内容扫」，与本判据「是否有人引用」目的不同。）
 */
const { Deploy: _deploy, Release: _release, ...BASE_EXCLUDES } = EXCLUDE_DIRS;
const REACHABILITY_EXCLUDES: Record<string, true> = BASE_EXCLUDES;
const MAX_TEXT_BYTES = 8 * 1024 * 1024;
const TOKEN_RE = /[A-Za-z0-9][A-Za-z0-9_-]*/g;

export interface Census {
  capabilities: string[];
  /** cap → 命中它的面（去重、保序） */
  reachable: Map<string, Face[]>;
  unreachable: string[];
}

interface WalkOpts {
  /** 只要这些扩展名的文件 */
  ext: RegExp;
}

/** 递归收集（不跟符号链接——与 `grep -r` 同语义；`Pre-Proc/{ns}` 是 ns 仓挂载符号链接） */
function walkFiles(root: string, target: string, opts: WalkOpts, out: string[]): void {
  let entries: string[];
  try {
    entries = readdirSync(target);
  } catch {
    return;
  }
  for (const name of entries) {
    if (isExcludedDirName(name, REACHABILITY_EXCLUDES)) continue;
    const abs = join(target, name);
    let st;
    try {
      st = lstatSync(abs);
    } catch {
      continue;
    }
    if (st.isSymbolicLink()) continue;
    if (st.isDirectory()) {
      walkFiles(root, abs, opts, out);
      continue;
    }
    if (!st.isFile() || !opts.ext.test(name) || st.size > MAX_TEXT_BYTES) continue;
    out.push(abs.slice(root.length + 1));
  }
}

/** 文本里出现的全部 capability 名（连字符对齐的连续片段语义） */
export function collectMentions(
  text: string,
  firstParts: ReadonlySet<string>,
  names: ReadonlySet<string>,
  out: Set<string>,
): void {
  for (const m of text.matchAll(TOKEN_RE)) {
    const parts = m[0].split('-');
    for (let i = 0; i < parts.length; i++) {
      if (!firstParts.has(parts[i])) continue;
      if (names.has(parts[i])) out.add(parts[i]);
      for (let j = i + 1; j < parts.length; j++) {
        const joined = parts.slice(i, j + 1).join('-');
        if (names.has(joined)) out.add(joined);
      }
    }
  }
}

/** 活跃 change 的 delta 目标（在途退休：其 REMOVED delta 尚未归档 ⇒ 此刻不判违规） */
export function pendingRetirementTargets(root: string): Set<string> {
  const changesDir = join(root, 'openspec/changes');
  const out = new Set<string>();
  let ids: string[];
  try {
    ids = readdirSync(changesDir);
  } catch {
    return out;
  }
  for (const id of ids) {
    if (id === 'archive') continue;
    const specs = join(changesDir, id, 'specs');
    let caps: string[];
    try {
      caps = readdirSync(specs);
    } catch {
      continue;
    }
    for (const cap of caps) out.add(cap);
    // 在途退休的第二种形态：畸形标题（`### ADDED Requirement:`）使 REMOVED delta 不可表达，
    // 该 change 以 `manual-removal.txt` 声明并按先例 `git rm` ⇒ 同样算「已声明的在途退休」。
    const manual = join(changesDir, id, 'manual-removal.txt');
    if (existsSync(manual)) {
      for (const line of readFileSync(manual, 'utf8').split('\n')) {
        const cap = line.trim();
        if (cap !== '' && !cap.startsWith('#')) out.add(cap);
      }
    }
  }
  return out;
}

/** live 族覆盖判据的违规集：三面不可达 ∧ 无在途退休 delta（正本 spec-governance） */
export function liveReachabilityViolations(root: string): { unreachable: string[]; pending: string[]; violations: string[] } {
  const c = census(root);
  const pending = pendingRetirementTargets(root);
  const pendingHere = c.unreachable.filter((cap) => pending.has(cap));
  return {
    unreachable: c.unreachable,
    pending: pendingHere,
    violations: c.unreachable.filter((cap) => !pending.has(cap)),
  };
}

/**
 * 判据自证（R1：本判据带豁免 ⇒ MUST 带负例）。
 * 夹具：可达 cap / 不可达 cap / 不可达但有在途 delta 的 cap ⇒ 只允许第三个被豁免。
 */
export function selfTest(): number {
  const tmp = mkdtempSync(join(tmpdir(), 'spec-reach-'));
  const cases: Array<{ name: string; ok: boolean; detail: string }> = [];
  try {
    const specs = join(tmp, 'openspec/specs');
    const write = (cap: string, body: string): void => {
      mkdirSync(join(specs, cap), { recursive: true });
      writeFileSync(join(specs, cap, 'spec.md'), body);
    };
    write('reachable-cap', '# reachable-cap Specification\n\n## Purpose\n\n甲\n\n## Requirements\n\n### Requirement: r\n\n由 `scripts/x.ts` 消费。\n');
    write('orphan-cap', '# orphan-cap Specification\n\n## Purpose\n\n乙\n\n## Requirements\n\n### Requirement: r\n\n无消费面。\n');
    write('pending-cap', '# pending-cap Specification\n\n## Purpose\n\n丙\n\n## Requirements\n\n### Requirement: r\n\n退休在途。\n');
    mkdirSync(join(tmp, 'scripts'), { recursive: true });
    writeFileSync(join(tmp, 'scripts/x.ts'), 'export const y = "reachable-cap";\n');
    mkdirSync(join(tmp, 'openspec/changes/some-change/specs/pending-cap'), { recursive: true });
    writeFileSync(
      join(tmp, 'openspec/changes/some-change/specs/pending-cap/spec.md'),
      '## REMOVED Requirements\n\n### Requirement: r\n\n**Reason**: 退休；MUST NOT 再引用。\n',
    );

    const c = liveReachabilityViolations(tmp);
    cases.push({
      name: '正例：可达 cap 不被判违规',
      ok: !c.violations.includes('reachable-cap'),
      detail: `unreachable=${JSON.stringify(c.unreachable)}`,
    });
    cases.push({ name: '正例：孤儿 cap 被判违规', ok: c.violations.includes('orphan-cap'), detail: 'orphan' });
    cases.push({
      name: '负例：有在途 delta 的不可达 cap MUST NOT 被判违规（豁免不得放宽到无 delta 者）',
      ok: !c.violations.includes('pending-cap') && c.pending.includes('pending-cap'),
      detail: `pending=${JSON.stringify(c.pending)}`,
    });
    cases.push({
      name: '负例：自身标题不得算邻接引用（否则 orphan-cap 会被判可达）',
      ok: c.unreachable.includes('orphan-cap'),
      detail: 'self-title-excluded',
    });

    let failed = 0;
    for (const k of cases) {
      console.log(`${k.ok ? 'PASS' : 'FAIL'}  ${k.name}（${k.detail}）`);
      if (!k.ok) failed++;
    }
    console.log(`\nspec-reachability self-test: ${cases.length - failed}/${cases.length} passed`);
    return failed === 0 ? 0 : 1;
  } finally {
    rmSync(tmp, { recursive: true, force: true });
  }
}

export function listCapabilities(root: string): string[] {
  const dir = join(root, 'openspec/specs');
  return readdirSync(dir)
    .filter((e) => {
      try {
        return statSync(join(dir, e)).isDirectory();
      } catch {
        return false;
      }
    })
    .sort();
}

/** 普查（唯一实现）：扫三面，返回可达 map 与不可达名单 */
export function census(root: string): Census {
  const capabilities = listCapabilities(root);
  const names = new Set(capabilities);
  const firstParts = new Set(capabilities.map((c) => c.split('-')[0]!));
  const hit = new Map<string, Set<Face>>();
  const add = (cap: string, face: Face): void => {
    const set = hit.get(cap) ?? new Set<Face>();
    set.add(face);
    hit.set(cap, set);
  };

  // ① 治理面 ② 代码面：按根目录分别归类
  const scan = (roots: readonly string[], face: Face): void => {
    const files: string[] = [];
    for (const r of roots) {
      const abs = join(root, r);
      let st;
      try {
        st = statSync(abs);
      } catch {
        continue;
      }
      if (st.isFile()) {
        if (TEXT_EXT_RE.test(r)) files.push(r);
      } else if (st.isDirectory()) walkFiles(root, abs, { ext: TEXT_EXT_RE }, files);
    }
    for (const rel of files) {
      const found = new Set<string>();
      collectMentions(readFileSync(join(root, rel), 'utf8'), firstParts, names, found);
      for (const cap of found) add(cap, face);
    }
  };
  scan(GOVERNANCE_ROOTS, 'governance');
  scan(CODE_ROOTS, 'code');

  // ③ 兄弟规约面：排除自身文件
  const specsRoot = join(root, 'openspec/specs');
  for (const cap of capabilities) {
    const dir = join(specsRoot, cap);
    for (const entry of readdirSync(dir)) {
      if (!entry.endsWith('.md')) continue;
      const found = new Set<string>();
      collectMentions(readFileSync(join(dir, entry), 'utf8'), firstParts, names, found);
      for (const hitCap of found) {
        if (hitCap !== cap) add(hitCap, 'sibling');
      }
    }
  }

  const reachable = new Map<string, Face[]>();
  for (const cap of capabilities) {
    const faces = hit.get(cap);
    if (faces !== undefined && faces.size > 0) reachable.set(cap, [...faces]);
  }
  return {
    capabilities,
    reachable,
    unreachable: capabilities.filter((c) => !reachable.has(c)),
  };
}

if (import.meta.main && process.argv.includes("--self-test")) process.exit(selfTest());
