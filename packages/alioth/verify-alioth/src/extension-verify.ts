/**
 * 扩展声明运行时验证（declared-but-unwired 检测）—— 对齐上游 `extension_verify.rs`（三层覆盖的
 * L2/L3 判定）与 vendored Gateway 加载器
 * （`packages/alioth/env-alioth/vendor/Framework/backend/runtime-engine/src/extension.rs:843-910`
 * `ExtensionLoader::load_from_dir`，其返回类型见 `runtime-contract/src/extension.rs:53-77`）。
 *
 * 判定强度（MUST NOT 软化）：只要存在**任一** uncovered 声明，整体 `status='degraded'`——
 * degraded ≠ passed。uncovered 的三类来源：
 * 1. `extensions/` 下存在加载器不认的文件名（永不加载）；
 * 2. 认可文件名但顶层形状不符（`Vec<T>` 期望数组 / `profiles.yaml` 期望 `{profiles:{}}`）；
 * 3. 条目缺反序列化必需键（缺键 = 反序列化失败 = Gateway `register_apps` 启动即退出）。
 *
 * `allowedForms` 由调用方注入（形态词汇表派生自 Gateway 加载器源码，本库不读 Rust）。
 * @module @dsh-alioth/verify-alioth/extension-verify
 */

import { mkdir, readFile, readdir, rename, rm, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { parse as parseYaml } from 'yaml'
import { artifactFingerprint } from './closure-audit.ts'

export const EXTENSION_VERIFY_SCHEMA_VERSION = '1.0'

/** 单条声明的覆盖状态。 */
export interface DeclarationCoverage {
  readonly id: string
  readonly file: string
  readonly form: string
  readonly status: 'wired' | 'uncovered'
  readonly evidence: string
}

/** 验证报告（canonical `extension-verify.json` 与降级 `extension-verify.degraded.json` 同结构）。 */
export interface ExtensionVerification {
  readonly schema_version: string
  readonly app: string
  readonly namespace: string
  readonly status: 'passed' | 'degraded'
  readonly covered: number
  readonly uncovered: number
  readonly declarations: readonly DeclarationCoverage[]
  /** 产物指纹（app.json + extensions/*.yaml 序联）——报告只对该指纹下的产物有效；不可得时为空串（不得当作通过证据）。 */
  readonly artifact_fingerprint: string
  readonly note: string
  readonly ts: string
}

/** 加载器认可的文件名 → 声明形态（形态名 = 文件族名，作为 injected `allowedForms` 的成员判据）。 */
interface FormSpec {
  readonly form: string
  readonly shape: 'array' | 'object'
  /** 条目的反序列化必需键（缺任一 = loader 反序列化失败）。 */
  readonly requiredKeys: readonly string[]
  /** 条目 id 的派生键优先级（取第一个非空字符串值的键）。 */
  readonly idKeys: readonly string[]
}

const FORM_SPECS: Record<string, FormSpec> = {
  'constraints.yaml': {
    form: 'constraints',
    shape: 'array',
    requiredKeys: ['entity', 'expression', 'message'],
    idKeys: ['entity'],
  },
  'rules.yaml': {
    form: 'rules',
    shape: 'array',
    requiredKeys: ['entity', 'name', 'trigger', 'condition', 'action'],
    idKeys: ['name', 'entity'],
  },
  'statemachines.yaml': {
    form: 'statemachines',
    shape: 'array',
    requiredKeys: ['entity', 'state_field', 'states', 'transitions', 'initial_state'],
    idKeys: ['entity'],
  },
  'workflows.yaml': {
    form: 'workflows',
    shape: 'array',
    requiredKeys: ['name', 'trigger', 'steps'],
    idKeys: ['name'],
  },
  'profiles.yaml': {
    form: 'profiles',
    shape: 'object',
    requiredKeys: ['profiles'],
    idKeys: ['name'],
  },
}

const KNOWN_FORMS: readonly string[] = Object.values(FORM_SPECS).map(spec => spec.form)

function entryId(entry: Record<string, unknown>, spec: FormSpec, fallback: string): string {
  for (const key of spec.idKeys) {
    const value = entry[key]
    if (typeof value === 'string' && value.trim() !== '') return value
  }
  return fallback
}

function missingRequiredKeys(entry: Record<string, unknown>, spec: FormSpec): readonly string[] {
  return spec.requiredKeys.filter(key => entry[key] === undefined || entry[key] === null)
}

/**
 * 扩展声明运行时验证。判定源 = `{appDir}/extensions/*.yaml`，逐条声明判定是否被
 * 加载器（真引擎装配路径）认可并可派生用例。
 */
export async function verifyExtensions(input: {
  readonly app: string
  readonly namespace: string
  readonly appDir: string
  readonly allowedForms: readonly string[]
}): Promise<ExtensionVerification> {
  const dir = path.join(path.resolve(input.appDir), 'extensions')
  const declarations: DeclarationCoverage[] = []
  // 产物指纹（复用 closure-audit 的唯一实现）：报告必须与它判定的产物绑定，否则
  // 「旧一次通过的 canonical」会在产物已改之后继续充当通过证据（上游
  // `extension_verify.rs:105-109` 的 `artifact_fingerprint` 字段同义）。
  // 产物未就绪（app.json 缺失）时按上游语义记空串 + note——**空指纹不得当作通过证据**。
  let fingerprint = ''
  let fingerprintNote = ''
  try {
    fingerprint = await artifactFingerprint(input.appDir)
  } catch (error) {
    fingerprintNote = `产物指纹不可得（${error instanceof Error ? error.message : String(error)}）：本报告未绑定产物，MUST NOT 作为 publish 通过证据`
  }

  const entries = await readdir(dir, { withFileTypes: true }).catch(() => null)
  const finalize = (note: string): ExtensionVerification => {
    const covered = declarations.filter(d => d.status === 'wired').length
    const uncovered = declarations.filter(d => d.status === 'uncovered').length
    return {
      schema_version: EXTENSION_VERIFY_SCHEMA_VERSION,
      app: input.app,
      namespace: input.namespace,
      status: uncovered > 0 ? 'degraded' : 'passed',
      covered,
      uncovered,
      declarations,
      artifact_fingerprint: fingerprint,
      note: fingerprintNote === '' ? note : `${note}；${fingerprintNote}`,
      ts: new Date().toISOString(),
    }
  }

  if (entries === null) {
    return finalize('extensions/ 目录不存在：无声明可验（未声明 ≠ 未覆盖，covered=0/uncovered=0）')
  }

  const yamlNames = entries
    .filter(entry => entry.isFile() && (entry.name.endsWith('.yaml') || entry.name.endsWith('.yml')))
    .map(entry => entry.name)
    .sort()

  if (yamlNames.length === 0) {
    return finalize('extensions/ 下无 .yaml 声明文件：无声明可验（covered=0/uncovered=0）')
  }

  for (const file of yamlNames) {
    const spec = FORM_SPECS[file]
    if (spec === undefined) {
      declarations.push({
        id: file,
        file,
        form: 'unknown',
        status: 'uncovered',
        evidence: `${file}: Gateway ExtensionLoader 只认 ${Object.keys(FORM_SPECS).join('/')}，不认的文件名永不加载（声明从未进入运行时）`,
      })
      continue
    }
    if (!input.allowedForms.includes(spec.form)) {
      declarations.push({
        id: file,
        file,
        form: spec.form,
        status: 'uncovered',
        evidence: `${file}: 形态 '${spec.form}' 不在注入的运行时词汇 allowedForms=[${input.allowedForms.join(',')}] 内——用例无法派生`,
      })
      continue
    }

    let document: unknown
    try {
      document = parseYaml(await readFile(path.join(dir, file), 'utf8')) as unknown
    } catch (error) {
      declarations.push({
        id: file,
        file,
        form: spec.form,
        status: 'uncovered',
        evidence: `${file}: YAML 解析失败（${error instanceof Error ? error.message : String(error)}）`,
      })
      continue
    }

    if (spec.shape === 'array') {
      if (!Array.isArray(document)) {
        declarations.push({
          id: file,
          file,
          form: spec.form,
          status: 'uncovered',
          evidence: `${file}: 顶层形状非数组（loader 期望 Vec<T>，反序列化失败 → Gateway register_apps 启动即退出）`,
        })
        continue
      }
      if (document.length === 0) continue
      document.forEach((item, index) => {
        const id = `${file}[${index}]`
        if (typeof item !== 'object' || item === null || Array.isArray(item)) {
          declarations.push({
            id,
            file,
            form: spec.form,
            status: 'uncovered',
            evidence: `${file}:${index} 条目非对象（类型 ${Array.isArray(item) ? 'array' : typeof item}）`,
          })
          return
        }
        const entry = item as Record<string, unknown>
        const missing = missingRequiredKeys(entry, spec)
        if (missing.length > 0) {
          declarations.push({
            id,
            file,
            form: spec.form,
            status: 'uncovered',
            evidence: `${file}:${index} 缺必需键 ${missing.join(', ')}（反序列化失败 → Gateway 启动即退出）`,
          })
          return
        }
        declarations.push({
          id: entryId(entry, spec, id),
          file,
          form: spec.form,
          status: 'wired',
          evidence: `${file}:${index} 形态与必需键齐备，已进入引擎装配路径`,
        })
      })
      continue
    }

    // profiles.yaml：顶层对象 `{profiles: {<name>: AppModelConfig}}`
    if (typeof document !== 'object' || document === null || Array.isArray(document)) {
      declarations.push({
        id: file,
        file,
        form: spec.form,
        status: 'uncovered',
        evidence: `${file}: 顶层形状非对象（loader 期望 {profiles: {...}}，反序列化失败 → Gateway 启动即退出）`,
      })
      continue
    }
    const profilesWrapper = (document as Record<string, unknown>)['profiles']
    if (typeof profilesWrapper !== 'object' || profilesWrapper === null || Array.isArray(profilesWrapper)) {
      declarations.push({
        id: file,
        file,
        form: spec.form,
        status: 'uncovered',
        evidence: `${file}: 缺 profiles 映射（或形态非对象）——loader 的 ProfilesWrapper 反序列化失败`,
      })
      continue
    }
    const profileEntries = Object.entries(profilesWrapper as Record<string, unknown>)
    if (profileEntries.length === 0) continue
    for (const [name, config] of profileEntries) {
      if (typeof config !== 'object' || config === null || Array.isArray(config)) {
        declarations.push({
          id: `${file}#${name}`,
          file,
          form: spec.form,
          status: 'uncovered',
          evidence: `${file}:profiles.${name} 条目非对象（AppModelConfig 反序列化失败）`,
        })
        continue
      }
      declarations.push({
        id: name,
        file,
        form: spec.form,
        status: 'wired',
        evidence: `${file}:profiles.${name} 形态齐备，已进入引擎装配路径`,
      })
    }
  }

  const covered = declarations.filter(d => d.status === 'wired').length
  const uncovered = declarations.filter(d => d.status === 'uncovered').length
  const formsSeen = [...new Set(yamlNames.map(name => FORM_SPECS[name]?.form ?? name))].join(',')
  return finalize(
    uncovered > 0
      ? `degraded：${uncovered} 条声明未被运行时覆盖（covered=${covered}，文件族=[${formsSeen}]，词汇 allowedForms=[${input.allowedForms.join(',')}]，已知形态=[${KNOWN_FORMS.join(',')}]）——未执行面 MUST NOT 判通过`
      : `passed：${covered} 条声明全部进入引擎装配路径（文件族=[${formsSeen}]）`,
  )
}

/**
 * 落盘验证报告。**降级不得占 canonical 位**（沿上游 `extension_verify.rs:26-27,126-134` 的两文件纪律）：
 * `status === 'passed'` → `{appDir}/extension-verify.json`；
 * `status === 'degraded'` → `{appDir}/extension-verify.degraded.json`，并**删除陈旧的 canonical**。
 *
 * 删除是必要的、不是洁癖：canonical 位只允许存在"对当前产物成立的真实通过"。若保留上一轮的
 * canonical passed，一次降级运行会让 publish 前置读到陈旧证据而静默放行；deferred 降级门的
 * 解除条件（canonical 存在/指针为 passed）也会立刻自我满足。删掉后，门只能由**下一次真实通过**
 * 解除，与上游 `deferred.rs:182-197` + `dialog_tools/verify_extensions.rs:36-37` 的意图一致。
 */
export async function writeExtensionVerify(appDir: string, result: ExtensionVerification): Promise<string> {
  const dir = path.resolve(appDir)
  await mkdir(dir, { recursive: true })
  const name = result.status === 'passed' ? 'extension-verify.json' : 'extension-verify.degraded.json'
  const target = path.join(dir, name)
  const tmp = path.join(dir, `.${name}.tmp-${process.pid}`)
  await writeFile(tmp, `${JSON.stringify(result, null, 2)}\n`, 'utf8')
  await rename(tmp, target)
  if (result.status !== 'passed') {
    await rm(path.join(dir, 'extension-verify.json'), { force: true })
  }
  return target
}
