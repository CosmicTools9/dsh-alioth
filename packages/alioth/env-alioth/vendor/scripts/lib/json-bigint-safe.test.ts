// json-bigint-safe 单测：>2^53 整数（Alioth 17 位 id）在 JSON 往返中不得被舍入
import { describe, expect, it } from 'bun:test';
import { parseBigIntSafe, rawIntegerText, restoreBigInts, stringifyBigIntSafe } from './json-bigint-safe';

const BLOCK_JSON = `{
  "id": "production-order",
  "coordinates": {
    "scene": { "code": "CD", "id": 16888498602639638 },
    "factor": { "code": "FRA", "id": 522417556774978450 },
    "function": { "code": "↓_GG", "id": 531424756029719824 }
  },
  "count": 3,
  "ratio": 1.25,
  "note": "id 字符串 522417556774978450 不应被改写"
}
`;

describe('parseBigIntSafe / stringifyBigIntSafe', () => {
  it('17 位 id 往返后逐位不变（回归：字段被舍入成 …400/…800）', () => {
    const out = stringifyBigIntSafe(parseBigIntSafe(BLOCK_JSON));
    expect(out).toContain('522417556774978450');
    expect(out).toContain('531424756029719824');
    expect(out).toContain('16888498602639638');
    expect(out).not.toContain('522417556774978400');
    expect(out).not.toContain('531424756029719800');
  });

  it('字符串内的数字与浮点/小整数不被触碰', () => {
    const out = stringifyBigIntSafe(parseBigIntSafe(BLOCK_JSON));
    expect(out).toContain('"note": "id 字符串 522417556774978450 不应被改写"');
    expect(out).toContain('"count": 3');
    expect(out).toContain('"ratio": 1.25');
  });

  it('二次往返幂等（哨兵不泄漏到产物文本）', () => {
    const once = stringifyBigIntSafe(parseBigIntSafe(BLOCK_JSON));
    const twice = stringifyBigIntSafe(parseBigIntSafe(once));
    expect(twice).toBe(once);
    expect(twice).not.toContain('ALIOTH_BIGINT');
  });

  it('变更其它字段后仍还原大整数（不依赖读写路径配对）', () => {
    const value = parseBigIntSafe(BLOCK_JSON) as {
      count: number;
      coordinates: { factor: { id: unknown } };
    };
    value.count = 9;
    value.coordinates.factor.id = 42;
    const out = stringifyBigIntSafe(value);
    expect(out).toContain('"count": 9');
    expect(out).toContain('531424756029719824');
    expect(out).not.toContain('ALIOTH_BIGINT');
  });

  it('负的大整数与安全边界内的整数分别处理', () => {
    const out = stringifyBigIntSafe(parseBigIntSafe('{"neg": -9007199254740993, "safe": 9007199254740991}'));
    expect(out).toContain('-9007199254740993');
    expect(out).toContain('9007199254740991');
  });

  it('缩进与尾换行符合仓库 JSON 写法（2 空格 + 尾 \\n）', () => {
    expect(stringifyBigIntSafe(parseBigIntSafe('{"a":{"b":1}}'))).toBe('{\n  "a": {\n    "b": 1\n  }\n}\n');
  });

  it('无大整数时 restoreBigInts 为恒等，解析结果不变', () => {
    expect(restoreBigInts('{"a":1}')).toBe('{"a":1}');
    expect(parseBigIntSafe('{"a":1}')).toEqual({ a: 1 });
  });

  it('rawIntegerText 暴露哨兵背后的原始数字文本（供 DB 比对，不经 Number）', () => {
    const value = parseBigIntSafe(BLOCK_JSON) as { coordinates: { factor: { id: unknown } }; ratio: number };
    expect(rawIntegerText(value.coordinates.factor.id)).toBe('522417556774978450');
    expect(rawIntegerText(value.ratio)).toBeNull();
    expect(rawIntegerText('普通字符串')).toBeNull();
  });
});
