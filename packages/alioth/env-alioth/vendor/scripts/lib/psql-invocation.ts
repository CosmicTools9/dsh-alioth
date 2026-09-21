/**
 * psql-invocation.ts — bash 命令 → psql / schema-info 的 **SQL 载荷**提取（内容可见性判定）
 *
 * 用途：OMP 运行期守卫（`.omp/extensions/{db-pipeline-guard,safety-guard}.ts`）在拦截前
 * 先解析**执行内容或执行文件**——只读查询 MUST NOT 因「通道形态可疑」被拦或询问，
 * 只有「明确保护的内容」（见 `scripts/lib/sql-scope.ts`）与「内容确实不可读」才进判定。
 *
 * 覆盖的载荷通道：
 *   - `psql -c "…"` / `--command[=]…`（含 `-tAc` 类合并短选项）
 *   - `psql -f <file>` / `--file=…`（读文件内容后判定）
 *   - `psql < file` / `psql <<'SQL' … SQL` / `psql <<< '…'`
 *   - `cat <file> | psql`、`echo/printf '…' | psql`（管道上游可读则读，变换流 → 不可读）
 *   - `mise run schema-info -- raw-sql "…"`
 *   - `bash -c "psql -c '…'"`（内层命令串递归一层）
 *
 * 本模块只做**提取与可读性**判定，不做 SQL 结构判定——后者统一归 `sql-scope.ts`。
 */
import { readFileSync, statSync } from 'node:fs';
import { isAbsolute, resolve } from 'node:path';

/** 单一载荷：`text` = SQL 文本（可能含 shell 展开 → 由 SQL 层判为不可判定） */
export interface SqlPayload {
  text: string;
  source: string;
}
export interface DbCommandScan {
  /** `psql` 处于命令行位的真调用（`which psql` / `psql --version` / `grep … psql` 不算） */
  psqlInvoked: boolean;
  /** 可读到的载荷（按出现顺序） */
  payloads: SqlPayload[];
  /** 内容不可读的通道描述（→ 无法证明安全，调用方一律驳回） */
  opaque: string[];
}

interface Word {
  t: 'word';
  v: string;
  /** 含 shell 展开（`$VAR` / `${…}` / `$(…)` / 反引号）→ 文本不完整 */
  dynamic: boolean;
}
interface Op {
  t: 'op';
  v: string;
  /** heredoc 体（`<<EOF … EOF`）：内容在命令文本内可见 */
  heredoc?: string;
}
type Tok = Word | Op;

const OPERATORS = new Set(['||', '&&', '|', ';', '&', '(', ')', '>', '>>', '<', '<<', '<<<', '<<-']);
/** 取独立值的 psql 短选项（解析合并短选项时用于定位 `-c` / `-f` 的位置） */
const PSQL_VALUE_SHORTS = new Set(['d', 'h', 'p', 'U', 'o', 'v', 'F', 'R', 'P', 'T', 'L']);
/** 读取文件载荷的上限（超出视为不可读：守卫只需要判定，不做流式） */
const MAX_FILE_BYTES = 4 * 1024 * 1024;

/** 命令词法：引号 / 转义 / shell 展开 / 注释 / 操作符 / heredoc（非解析器替代，是载荷提取的输入层） */
function tokenize(cmd: string): Tok[] {
  const toks: Tok[] = [];
  let i = 0;
  const n = cmd.length;
  while (i < n) {
    const c = cmd[i];
    if (c === ' ' || c === '\t' || c === '\r') {
      i++;
      continue;
    }
    if (c === '\n') {
      toks.push({ t: 'op', v: ';' });
      i++;
      continue;
    }
    if (c === '#') {
      while (i < n && cmd[i] !== '\n') i++;
      continue;
    }
    const op3 = cmd.slice(i, i + 3);
    if (op3 === '<<<' || op3 === '<<-') {
      toks.push({ t: 'op', v: op3 });
      i += 3;
      continue;
    }
    const op2 = cmd.slice(i, i + 2);
    if (op2 === '||' || op2 === '&&' || op2 === '>>' || op2 === '<<') {
      if (op2 === '<<') {
        const [delim, body, next] = readHeredoc(cmd, i + 2, op3 === '<<-');
        toks.push({ t: 'op', v: '<<', heredoc: body });
        toks.push({ t: 'word', v: delim, dynamic: false });
        i = next;
        continue;
      }
      toks.push({ t: 'op', v: op2 });
      i += 2;
      continue;
    }
    if (OPERATORS.has(c)) {
      toks.push({ t: 'op', v: c });
      i++;
      continue;
    }
    const [word, next] = readWord(cmd, i);
    toks.push(word);
    i = next;
  }
  return toks;
}

/** 读一个 heredoc：`<<[-]DELIM` 后的行直到 DELIM 行（`<<-` 去掉行首 tab） */
function readHeredoc(cmd: string, from: number, stripTabs: boolean): [string, string, number] {
  let i = from;
  while (i < cmd.length && (cmd[i] === ' ' || cmd[i] === '\t')) i++;
  const [delimWord, afterDelim] = readWord(cmd, i);
  const delim = delimWord.v;
  const nl = cmd.indexOf('\n', afterDelim);
  if (nl === -1) return [delim, '', cmd.length];
  const lines = cmd.slice(nl + 1).split('\n');
  const body: string[] = [];
  let consumed = nl + 1;
  for (const line of lines) {
    const probe = stripTabs ? line.replace(/^\t+/, '') : line;
    consumed += line.length + 1;
    if (probe === delim) return [delim, body.join('\n'), Math.min(consumed, cmd.length)];
    body.push(line);
  }
  return [delim, body.join('\n'), cmd.length];
}

/** 读一个 shell 词：引号去壳、转义还原、展开标记 */
function readWord(cmd: string, from: number): [Word, number] {
  let i = from;
  let v = '';
  let dynamic = false;
  const n = cmd.length;
  const isDelim = (ch: string) => ch === undefined || /[\s;|&<>()]/.test(ch);
  while (i < n && !isDelim(cmd[i])) {
    const c = cmd[i];
    if (c === "'") {
      const end = cmd.indexOf("'", i + 1);
      v += cmd.slice(i + 1, end === -1 ? n : end);
      i = end === -1 ? n : end + 1;
      continue;
    }
    if (c === '"') {
      i++;
      while (i < n && cmd[i] !== '"') {
        if (cmd[i] === '\\' && i + 1 < n) {
          const nx = cmd[i + 1];
          if (!/[\\$"`]/.test(nx)) v += '\\';
          v += nx;
          i += 2;
          continue;
        }
        if (cmd[i] === '$') dynamic = true;
        v += cmd[i++];
      }
      i++;
      continue;
    }
    if (c === '\\' && i + 1 < n) {
      v += cmd[i + 1];
      i += 2;
      continue;
    }
    if (c === '`') {
      dynamic = true;
      const end = cmd.indexOf('`', i + 1);
      v += cmd.slice(i, end === -1 ? n : end + 1);
      i = end === -1 ? n : end + 1;
      continue;
    }
    if (c === '$') {
      dynamic = true;
      if (cmd[i + 1] === '(') {
        let depth = 0;
        let j = i + 1;
        for (; j < n; j++) {
          if (cmd[j] === '(') depth++;
          else if (cmd[j] === ')') {
            depth--;
            if (depth === 0) break;
          }
        }
        v += cmd.slice(i, Math.min(j + 1, n));
        i = Math.min(j + 1, n);
        continue;
      }
      if (cmd[i + 1] === '{') {
        const end = cmd.indexOf('}', i);
        v += cmd.slice(i, end === -1 ? n : end + 1);
        i = end === -1 ? n : end + 1;
        continue;
      }
    }
    v += c;
    i++;
  }
  return [{ t: 'word', v, dynamic }, i];
}

interface Command {
  words: Word[];
  redirects: Array<{ op: string; target: string; dynamic: boolean; heredoc?: string }>;
  pipedFrom?: Command;
}

function splitCommands(toks: Tok[]): Command[] {
  const cmds: Command[] = [];
  let cur: Command | null = null;
  let lastPipe: Command | null = null;
  for (let i = 0; i < toks.length; i++) {
    const tk = toks[i];
    if (tk.t === 'word') {
      if (!cur) cur = { words: [], redirects: [] };
      cur.words.push(tk);
      continue;
    }
    if (tk.v === '>' || tk.v === '>>' || tk.v === '<' || tk.v === '<<' || tk.v === '<<<') {
      if (!cur) cur = { words: [], redirects: [] };
      const next = toks[i + 1];
      const target = next?.t === 'word' ? next.v : '';
      const dynamic = next?.t === 'word' ? next.dynamic : false;
      cur.redirects.push({ op: tk.v, target, dynamic, heredoc: tk.heredoc });
      if (next?.t === 'word') i++;
      continue;
    }
    if (tk.v === '|') {
      if (cur) {
        cmds.push(cur);
        lastPipe = cur;
      }
      cur = { words: [], redirects: [], pipedFrom: lastPipe ?? undefined };
      continue;
    }
    if (cur) cmds.push(cur);
    cur = null;
    lastPipe = null;
  }
  if (cur) cmds.push(cur);
  return cmds;
}

function commandName(words: Word[]): { name: string; rest: Word[] } {
  let i = 0;
  while (i < words.length && /^[A-Za-z_][A-Za-z0-9_]*=/.test(words[i].v)) i++; // VAR=x 赋值前缀
  if (words[i]?.v === 'env') {
    i++;
    while (i < words.length && (/^[A-Za-z_][A-Za-z0-9_]*=/.test(words[i].v) || words[i].v.startsWith('-'))) i++;
  }
  const raw = words[i]?.v ?? '';
  return { name: raw.split('/').pop() ?? raw, rest: words.slice(i + 1) };
}

function readTextFile(path: string, cwd: string): string | null {
  const abs = isAbsolute(path) ? path : resolve(cwd, path);
  try {
    if (!statSync(abs).isFile() || statSync(abs).size > MAX_FILE_BYTES) return null;
    return readFileSync(abs, 'utf8');
  } catch {
    return null;
  }
}

/** `"$(cat f)"` / `"$(<f)"` 形态：内容在可读文件里 → 直接读文件 */
function derefCommandSubstitution(text: string): { path: string } | null {
  const m = /^\$\(\s*(?:cat\s+|<\s*)([^\s)]+)\s*\)$/.exec(text.trim());
  return m ? { path: m[1] } : null;
}

/** 管道上游可读性解析：`cat f | psql` 读文件；`grep/awk/… | psql` 是变换流 → 不可读 */
function pipeSourcePayloads(
  src: Command | undefined,
  cwd: string,
): { payloads: SqlPayload[]; opaque: string[] } {
  const payloads: SqlPayload[] = [];
  const opaque: string[] = [];
  if (!src) return { payloads, opaque };
  const { name, rest } = commandName(src.words);
  const files = (skipValueAfter: string[]): string[] => {
    const out: string[] = [];
    for (let i = 0; i < rest.length; i++) {
      const w = rest[i].v;
      if (w.startsWith('-')) {
        if (skipValueAfter.includes(w) && i + 1 < rest.length) i++;
        continue;
      }
      out.push(w);
    }
    return out;
  };
  if (name === 'cat' || name === 'bat' || name === 'less' || name === 'more') {
    const fs = files([]);
    if (fs.length === 0) return { payloads, opaque: [`${name}（无文件实参，内容不可见）`] };
    for (const f of fs) {
      const text = readTextFile(f, cwd);
      if (text === null) opaque.push(`${name} ${f}（文件不可读）`);
      else payloads.push({ text, source: `${name} ${f} → psql` });
    }
    return { payloads, opaque };
  }
  if (name === 'head' || name === 'tail') {
    const fs = files(['-n', '-c']);
    if (fs.length === 0) return { payloads, opaque: [`${name}（无文件实参，内容不可见）`] };
    for (const f of fs) {
      const text = readTextFile(f, cwd);
      if (text === null) opaque.push(`${name} ${f}（文件不可读）`);
      else payloads.push({ text, source: `${name} ${f} → psql` });
    }
    return { payloads, opaque };
  }
  if (name === 'echo' || name === 'printf') {
    const texts = rest.filter((w) => !w.v.startsWith('-'));
    if (texts.length === 0) return { payloads, opaque: [`${name}（无实参）`] };
    // 逐词 + 整串都作为载荷（`printf '%s' 'DROP TABLE …'` 的格式串不可当 SQL 解析，
    // 而 SQL 文本可能是其中任一实参；判定取最严重结论，重复载荷无副作用）
    payloads.push({ text: texts.map((w) => w.v).join(' '), source: `${name} → psql` });
    for (const w of texts) payloads.push({ text: w.v, source: `${name} 实参 → psql` });
    return { payloads, opaque };
  }
  return { payloads, opaque: [`${name || '上游命令'} → psql（变换流，内容不可见）`] };
}

/** `psql` 实参 → 载荷（`-c` / `--command` / `-f` / `--file` / 重定向 / 管道上游） */
function psqlPayloads(cmd: Command, cwd: string): { payloads: SqlPayload[]; opaque: string[]; readOnlyForm: boolean } {
  const payloads: SqlPayload[] = [];
  const opaque: string[] = [];
  const words = commandName(cmd.words).rest;
  let readOnlyForm = false;
  let inlineFound = false;
  const consume = (raw: string, dynamic: boolean, source: string) => {
    inlineFound = true;
    const deref = dynamic ? derefCommandSubstitution(raw) : null;
    if (deref) {
      const text = readTextFile(deref.path, cwd);
      if (text === null) opaque.push(`${source}（$(cat ${deref.path}) 文件不可读）`);
      else payloads.push({ text, source: `${source}（$(cat ${deref.path})）` });
      return;
    }
    payloads.push({ text: raw, source });
  };
  for (let i = 0; i < words.length; i++) {
    const w = words[i];
    if (w.v === '--version' || w.v === '-V' || w.v === '--help' || w.v === '-?') readOnlyForm = true;
    if (w.v === '--command' || w.v === '--file') {
      const isFile = w.v === '--file';
      const val = words[i + 1];
      if (!val) {
        opaque.push(`${w.v}（缺实参）`);
        continue;
      }
      i++;
      if (isFile) {
        inlineFound = true;
        if (val.v === '-') opaque.push('psql --file -（stdin 不可见）');
        else {
          const text = readTextFile(val.v, cwd);
          if (text === null) opaque.push(`psql -f ${val.v}（文件不可读）`);
          else payloads.push({ text, source: `-f ${val.v}` });
        }
      } else consume(val.v, val.dynamic, '-c 内联');
      continue;
    }
    const longEq = /^--(command|file)=(.*)$/s.exec(w.v);
    if (longEq) {
      if (longEq[1] === 'file') {
        inlineFound = true;
        const text = readTextFile(longEq[2], cwd);
        if (text === null) opaque.push(`psql -f ${longEq[2]}（文件不可读）`);
        else payloads.push({ text, source: `-f ${longEq[2]}` });
      } else consume(longEq[2], w.dynamic, '-c 内联');
      continue;
    }
    if (w.v.startsWith('-') && !w.v.startsWith('--') && w.v.length > 1) {
      const cluster = w.v.slice(1);
      for (let j = 0; j < cluster.length; j++) {
        const ch = cluster[j];
        const rest = cluster.slice(j + 1);
        if (ch === 'c' || ch === 'f') {
          let val = rest;
          if (!val) {
            const next = words[i + 1];
            if (!next) break;
            i++;
            val = next.v;
          }
          if (ch === 'f') {
            inlineFound = true;
            if (val === '-') opaque.push('psql -f -（stdin 不可见）');
            else {
              const text = readTextFile(val, cwd);
              if (text === null) opaque.push(`psql -f ${val}（文件不可读）`);
              else payloads.push({ text, source: `-f ${val}` });
            }
          } else {
            consume(val, w.dynamic, '-c 内联');
          }
          break;
        }
        if (PSQL_VALUE_SHORTS.has(ch)) {
          if (!rest) i++; // 取独立值的选项：吃掉下一个词，避免误当载荷
          break;
        }
      }
    }
  }
  for (const r of cmd.redirects) {
    if (r.op === '<') {
      inlineFound = true;
      const text = readTextFile(r.target, cwd);
      if (text === null) opaque.push(`psql < ${r.target}（文件不可读）`);
      else payloads.push({ text, source: `< ${r.target}` });
    } else if (r.op === '<<') {
      inlineFound = true;
      payloads.push({ text: r.heredoc ?? '', source: 'heredoc' });
    } else if (r.op === '<<<') {
      inlineFound = true;
      if (r.dynamic) opaque.push('psql <<< "$…"（here-string 含展开，内容不可见）');
      else payloads.push({ text: r.target, source: 'here-string' });
    }
  }
  if (!inlineFound && cmd.pipedFrom) {
    const up = pipeSourcePayloads(cmd.pipedFrom, cwd);
    payloads.push(...up.payloads);
    opaque.push(...up.opaque);
  } else if (!inlineFound && !cmd.pipedFrom) {
    opaque.push('psql（交互式，未提供 -c/-f/重定向 → stdin 内容不可见）');
  }
  return { payloads, opaque, readOnlyForm };
}

/** schema-info `-- raw-sql` 载荷（Meta 后端实时查询通道） */
function rawSqlPayloads(cmd: Command): SqlPayload[] {
  const words = commandName(cmd.words).rest;
  const i = words.findIndex((w) => w.v === 'raw-sql');
  if (i === -1) return [];
  const sql = words[i + 1];
  return sql ? [{ text: sql.v, source: 'schema-info -- raw-sql' }] : [];
}

/**
 * 扫描 bash 命令 → psql / raw-sql 载荷与不可读通道。
 *
 * `cwd` 用于解析相对文件路径（调用方传 hook 上下文的 `ctx.cwd`）。
 */
export function scanDbCommand(command: string, cwd: string, depth = 0): DbCommandScan {
  const out: DbCommandScan = { psqlInvoked: false, payloads: [], opaque: [] };
  for (const cmd of splitCommands(tokenize(command))) {
    const { name } = commandName(cmd.words);
    if (name === 'psql') {
      const r = psqlPayloads(cmd, cwd);
      if (r.readOnlyForm) continue; // `psql --version` / `-V` / `--help`：只读形态，不触发守卫
      out.psqlInvoked = true;
      out.payloads.push(...r.payloads);
      out.opaque.push(...r.opaque);
      continue;
    }
    if (name === 'mise') {
      const raws = rawSqlPayloads(cmd);
      out.payloads.push(...raws);
      continue;
    }
    // `bash -c "psql …"`：内层命令串是可见文本 → 递归一层（包装不构成不可见通道）
    if (['sh', 'bash', 'zsh', 'dash', 'ksh'].includes(name) && depth < 2) {
      const words = commandName(cmd.words).rest;
      const idx = words.findIndex((w) => w.v === '-c');
      const inner = idx === -1 ? undefined : words.slice(idx + 1).find((w) => !w.v.startsWith('-'));
      if (inner) {
        const sub = scanDbCommand(inner.v, cwd, depth + 1);
        if (sub.psqlInvoked) {
          out.psqlInvoked = true;
          out.payloads.push(...sub.payloads.map((p) => ({ ...p, source: `sh -c: ${p.source}` })));
          out.opaque.push(...sub.opaque.map((o) => `sh -c: ${o}`));
        }
      }
    }
  }
  return out;
}
