/**
 * rust-test-regions.ts — Rust 源码「字面量掩码 + #[cfg(test)] 模块段」共享工具
 *
 * 消费方（单一实现，禁第二份）：
 *   - `check-secrets-staged.ts`：凭据模式命中位于内联 `#[cfg(test)]` 段时不判违规
 *     （测试夹具弃置密钥为合法形态——与既有 tests/ 目录豁免同源；生产代码仍全量扫）
 *   - `check-appagent-observability.ts`：env 旋钮代码面扫描排除内联测试段
 *
 * 逐字迁移自 `check-appagent-observability.ts` 原实现。
 */

/**
 * 逐字符扫描 Rust 源码（字符串字面量提取，非结构解析）：
 * 跳过行注释/嵌套块注释，收集字符串字面量（含 `r#"…"#` raw string）并记录起始行号；
 * 同时产出「字面量与注释被抹空、行号/大括号保留」的掩码文本，供 `#[cfg(test)]` 区间判定用
 * （避免字符串里的大括号干扰配平）。
 */
export function scanRust(src: string): { literals: { value: string; line: number }[]; masked: string } {
  const mask = src.split('');
  const literals: { value: string; line: number }[] = [];
  const blank = (from: number, to: number) => {
    for (let t = from; t < to; t++) if (mask[t] !== '\n') mask[t] = ' ';
  };
  const n = src.length;
  let i = 0;
  let line = 1;
  const adv = (count = 1) => {
    for (let t = 0; t < count && i < n; t++) {
      if (src[i] === '\n') line++;
      i++;
    }
  };
  while (i < n) {
    const c = src[i];
    if (c === '/' && src[i + 1] === '/') {
      const start = i;
      while (i < n && src[i] !== '\n') adv();
      blank(start, i);
      continue;
    }
    if (c === '/' && src[i + 1] === '*') {
      const start = i;
      let depth = 1;
      adv(2);
      while (i < n && depth > 0) {
        if (src[i] === '/' && src[i + 1] === '*') {
          depth++;
          adv(2);
        } else if (src[i] === '*' && src[i + 1] === '/') {
          depth--;
          adv(2);
        } else adv();
      }
      blank(start, i);
      continue;
    }
    if (c === 'r' && (src[i + 1] === '"' || src[i + 1] === '#')) {
      // raw string: r"…" | r#"…"# | r##"…"##
      let j = i + 1;
      let hashes = 0;
      while (src[j] === '#') {
        hashes++;
        j++;
      }
      if (src[j] === '"') {
        const close = '"' + '#'.repeat(hashes);
        const end = src.indexOf(close, j + 1);
        if (end !== -1) {
          const startLine = line;
          literals.push({ value: src.slice(j + 1, end), line: startLine });
          blank(i, end + close.length);
          adv(end + close.length - i);
          continue;
        }
      }
    }
    if (c === '"') {
      const startLine = line;
      let j = i + 1;
      let value = '';
      while (j < n) {
        const d = src[j];
        if (d === '\\') {
          value += src[j + 1] ?? '';
          j += 2;
          continue;
        }
        if (d === '"') break;
        value += d;
        j++;
      }
      literals.push({ value, line: startLine });
      blank(i, j + 1);
      adv(j + 1 - i);
      continue;
    }
    adv();
  }
  return { literals, masked: mask.join('') };
}

/** `#[cfg(test)]` 模块段行号集（行级启发：属性行起，首个 `{` 后配平至深度归零为止）。 */
export function cfgTestLines(masked: string): Set<number> {
  const skipped = new Set<number>();
  const lines = masked.split('\n');
  let inTest = false;
  let depth = 0;
  let started = false;
  for (let li = 0; li < lines.length; li++) {
    const text = lines[li];
    if (!inTest && /#\[\s*cfg\s*\(\s*test\s*\)\s*\]/.test(text)) {
      inTest = true;
      depth = 0;
      started = false;
    }
    if (!inTest) continue;
    skipped.add(li + 1);
    for (const ch of text) {
      if (ch === '{') {
        depth++;
        started = true;
      } else if (ch === '}') depth--;
    }
    if (started && depth <= 0) inTest = false;
  }
  return skipped;
}
