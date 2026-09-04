#!/usr/bin/env bun
// check-config-json.mjs — 校验结构化配置文件内容合法性（gate 内容 schema 校验）
//
// 用法: bun scripts/check/check-config-json.mjs <type> <json 路径>
//   type ∈ { module, block, service }
// 退出码: 0 = 合法；1 = 不合法（含原因输出）；2 = 用法错误
//
// 服务对象: AppAgent 技能执行 gate（alioth-module 1.1 / alioth-block 1.1 /
// alioth-service 1.1）——拦截 LLM 写坏配置（原型设计中间态 JSON、id 命名违规、
// 必填字段缺失），防止坏产物进入 PlatformCatalog 自我强化。
//
// 校验实现（JSON.parse + 字段断言，禁止正则模拟解析器——NO_REGEX_FOR_PARSING）:
//   module  : 必填 id/namespace/name/version/status/routePrefix；id kebab-case
//             且不含 "-app" 后缀；routePrefix 以 / 开头；不得携带设计中间态字段
//             （schema/stage/preflight/prototype/mock/coverage）
//   block   : 必填 id/name/version；id kebab-case 且不含 "-app" 后缀（namespace 路径权威，可选）
//   service : 必填 id/namespace/version/aliothVersion；id kebab-case
//             （命名域惯例 {domain}-service）

import { readFileSync } from 'node:fs';

const type = process.argv[2];
const srcFile = process.argv[3];
if (!['module', 'block', 'service'].includes(type) || !srcFile) {
  console.error('usage: bun check-config-json.mjs <module|block|service> <json 路径>');
  process.exit(2);
}

let raw;
try {
  raw = readFileSync(srcFile, 'utf-8');
} catch (e) {
  console.error(`❌ 无法读取 ${srcFile}: ${e.message}`);
  process.exit(1);
}

// 1. JSON 可解析 + 顶层对象
let conf;
try {
  conf = JSON.parse(raw);
} catch (e) {
  console.error(`❌ ${type}.json 不是合法 JSON: ${e.message}`);
  process.exit(1);
}
if (typeof conf !== 'object' || conf === null || Array.isArray(conf)) {
  console.error(`❌ ${type}.json 顶层必须是 JSON 对象`);
  process.exit(1);
}

const errors = [];

// 2. 必填字段（按类型）
const required = {
  module: ['id', 'namespace', 'name', 'version', 'status', 'routePrefix'],
  block: ['id', 'name', 'version'],
  service: ['id', 'namespace', 'version'],
}[type];
for (const key of required) {
  if (typeof conf[key] !== 'string' || conf[key].trim() === '') {
    errors.push(`缺少必填字段: ${key}`);
  }
}
// aliothVersion：仅 Alioth/AVIC-CAASEC 必填（SERVICE_SPEC §2.2——WZ 暂不设置）
if (
  type === 'service' &&
  ['Alioth', 'AVIC-CAASEC'].includes(conf.namespace) &&
  (typeof conf.aliothVersion !== 'string' || conf.aliothVersion.trim() === '')
) {
  errors.push('缺少必填字段: aliothVersion（Alioth/AVIC-CAASEC 必填）');
}
// block.services：BLOCK_SCHEMA 必填（string[]，可为空数组）——独立校验（required 循环按 string 处理）
if (type === 'block' && !Array.isArray(conf.services)) {
  errors.push('缺少必填字段: services（须为数组）');
}

// 3. id 命名规约（kebab-case，模块/Block 禁止 "-app" 后缀）
const id = conf.id;
if (typeof id === 'string' && id.trim() !== '') {
  if (id.endsWith('-app')) {
    errors.push(
      `id "${id}" 携带 "-app" 后缀——模块/Block id 属于业务命名域（如 inventory），App code 才使用 -app 命名域`,
    );
  }
  if (id.startsWith('-') || id.endsWith('-')) {
    errors.push(`id "${id}" 以连字符开头/结尾（kebab-case 禁止）`);
  }
  for (const ch of id) {
    const ok = (ch >= 'a' && ch <= 'z') || (ch >= '0' && ch <= '9') || ch === '-';
    if (!ok) {
      errors.push(`id "${id}" 含非法字符 "${ch}"（kebab-case: 小写字母/数字/连字符）`);
      break;
    }
  }
}

// 4. module 专属语义
if (type === 'module') {
  if (typeof conf.routePrefix === 'string' && conf.routePrefix !== '' && !conf.routePrefix.startsWith('/')) {
    errors.push(`routePrefix "${conf.routePrefix}" 必须以 / 开头`);
  }
  // blockAssembly 形态契约（fix-appagent-structure-layout）：必须为对象
  // （mode/blocks 必备，blocks 每项含 id/label/group/order），数组形态是
  // 链路历史错误结构——门禁拒绝
  const ba = conf.blockAssembly;
  if (ba !== undefined) {
    if (Array.isArray(ba) || typeof ba !== 'object' || ba === null) {
      errors.push(
        'blockAssembly 必须是 JSON 对象（mode/blocks/serviceBindings），禁止数组形态',
      );
    } else {
      if (ba.mode !== 'multi-block' && ba.mode !== 'single-block') {
        errors.push(`blockAssembly.mode "${ba.mode}" 非法（multi-block | single-block）`);
      }
      if (!Array.isArray(ba.blocks) || ba.blocks.length === 0) {
        errors.push('blockAssembly.blocks 必须为非空数组');
      } else {
        ba.blocks.forEach((b, i) => {
          for (const k of ['id', 'label', 'group', 'order']) {
            if (!(k in b)) errors.push(`blockAssembly.blocks[${i}] 缺少 "${k}"`);
          }
        });
      }
    }
  }
  // 拦截 LLM 误写原型设计中间态 JSON
  for (const key of ['schema', 'stage', 'preflight', 'prototype', 'mock', 'coverage']) {
    if (key in conf) {
      errors.push(`携带设计中间态字段 "${key}"（${type}.json 是配置，不含原型设计元数据）`);
    }
  }
}

// 5. service 专属语义
if (type === 'service') {
  if (typeof conf.version === 'string' && conf.version !== '') {
    const parts = conf.version.split('.');
    if (parts.length < 2 || parts.some((p) => p.trim() === '' || Number.isNaN(Number(p)))) {
      errors.push(`version "${conf.version}" 不是合法 semver（需 x.y.z 数字段）`);
    }
  }
  // ontology.entities 结构契约（fix-appagent-structure-layout）：
  // 每实体必须有 table 与 coordinates 对象（键存在即可——坐标值可待 coord
  // 问答回填，但键结构 MUST 齐备，杜绝链路生成时结构缺字段）
  if (Array.isArray(conf.ontology?.entities)) {
    conf.ontology.entities.forEach((e, i) => {
      if (typeof e.table !== 'string' || e.table.trim() === '') {
        errors.push(`ontology.entities[${i}] 缺少 table`);
      }
      if (typeof e.coordinates !== 'object' || e.coordinates === null) {
        errors.push(`ontology.entities[${i}] 缺少 coordinates 对象（scene/factor/function 键）`);
      } else {
        for (const k of ['scene', 'factor', 'function']) {
          if (!(k in e.coordinates)) {
            errors.push(`ontology.entities[${i}].coordinates 缺少键 "${k}"`);
          }
        }
      }
    });
  }
  if (conf.dtoDependencies !== undefined && !Array.isArray(conf.dtoDependencies)) {
    errors.push('dtoDependencies 必须为数组');
  }
}

if (errors.length > 0) {
  console.error(`❌ ${type}.json 内容校验失败（${srcFile}）:`);
  for (const e of errors) {
    console.error(`  - ${e}`);
  }
  process.exit(1);
}

console.log(`✅ ${type}.json 内容校验通过: ${srcFile}`);
process.exit(0);
