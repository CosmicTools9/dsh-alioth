/**
 * 受控并行分派原语（契约 §5.2）——管线里唯一允许并行的形态，四条纪律：
 *
 * - **显式登记**：只跑调用方登记的单元（无隐式发现、无动态 fan-out）；id 必须唯一。
 * - **共享写面命中即串行**：写面命中 `sharedWriteSurfaces` 的单元**独占执行**（单所有者），
 *   不与任何单元重叠。清单/注册面（`app.json`/`module.json`/`extensions/*.yaml`/`Cargo.toml`）
 *   是全局状态，多写者必然互相覆盖。写面为空的单元同样按独占处理——未知写面不得并行。
 *   两个单元写同一条私有路径也按共享处理（同一写面只有一个所有者）。
 * - **并发度默认 1**：显式传参才放开；1 时退化为确定性顺序执行。
 * - **结果确定性排序**：按登记序返回，与完成先后无关；失败收敛为 `ok=false`，
 *   失败单元与因失败未启动的单元一起进 `incomplete`（显式未完成清单，绝不静默丢弃）。
 *
 * 唯一的系统状态是调度本身：单元内部不得共享可变状态（每个单元只写自己声明的写面）。
 * @module @dsh-alioth/tool-alioth-orchestrator/parallel
 */

/** 一个显式登记的分派单元。 */
export interface ParallelUnit<T> {
  /** 登记序内唯一的单元 id（重复即 throw）。 */
  readonly id: string
  /** 该单元会写的路径（相对/绝对皆可，模板须由调用方先行解析）。空 = 未知写面（独占）。 */
  readonly writes: readonly string[]
  readonly run: () => Promise<T>
}

/** 单单元结果（`status='skipped'` = 因前序失败未启动）。 */
interface ParallelUnitResult<T> {
  readonly id: string
  readonly status: 'ok' | 'failed' | 'skipped'
  /** 该单元被判独占（命中共享写面 / 未知写面 / 与他单元同写一路径）。 */
  readonly exclusive: boolean
  readonly value?: T
  readonly error?: string
}

/** 一次受控并行的整体结果。 */
export interface ParallelOutcome<T> {
  readonly ok: boolean
  /** 按登记序排列（与完成先后无关）。 */
  readonly results: readonly ParallelUnitResult<T>[]
  readonly concurrency: number
  /** 显式未完成清单：失败单元 + 因失败未启动的单元（登记序）。 */
  readonly incomplete: readonly string[]
  /** 被判独占的单元 id（登记序）——串行化依据的审计面。 */
  readonly exclusive: readonly string[]
}

/**
 * 管线共享写面（契约 §5.2 枚举）：清单面（`app.json`/`module.json`）、Gateway 扩展注册面
 * （`extensions/*.yaml`）、命名空间工作区清单（`Cargo.toml`）。命中即串行。
 */
export const PIPELINE_SHARED_SURFACES: readonly string[] = [
  'app.json',
  'module.json',
  'extensions/*.yaml',
  'Cargo.toml',
]

function normalizeWrite(target: string): string {
  const slashed = target.replace(/\\/g, '/').replace(/\/{2,}/g, '/').replace(/^\.\//, '')
  return slashed.endsWith('/') ? slashed.slice(0, -1) : slashed
}

/** 路径段模式 → 正则：`*` 匹配任意非分隔符串，`?` 匹配单字符，其余字面量。 */
function segmentRegExp(segment: string): RegExp {
  const source = segment
    .replace(/[.+^${}()|[\]\\]/g, '\\$&')
    .replace(/\*/g, '[^/]*')
    .replace(/\?/g, '[^/]')
  return new RegExp(`^${source}$`)
}

/**
 * 写面模式是否覆盖某条写入路径：**后缀逐段**匹配（模式不必覆盖整条路径），
 * 段内 `*`/`?` 支持通配。`a.json` 命中 `Apps/x/a.json`，但不命中 `Apps/x/a.json.bak`。
 */
export function pathTouchesSurface(write: string, surface: string): boolean {
  const writeSegments = normalizeWrite(write).split('/').filter(segment => segment !== '')
  const surfaceSegments = normalizeWrite(surface).split('/').filter(segment => segment !== '')
  if (surfaceSegments.length === 0 || surfaceSegments.length > writeSegments.length) {
    return false
  }
  const tail = writeSegments.slice(writeSegments.length - surfaceSegments.length)
  return surfaceSegments.every((segment, index) => segmentRegExp(segment).test(tail[index] ?? ''))
}

/** 被两个及以上单元写到的路径（同一写面只能有一个所有者）。 */
function duplicatedWrites<T>(units: readonly ParallelUnit<T>[]): ReadonlySet<string> {
  const counts = new Map<string, number>()
  for (const unit of units) {
    for (const write of unit.writes) {
      const key = normalizeWrite(write)
      counts.set(key, (counts.get(key) ?? 0) + 1)
    }
  }
  return new Set([...counts.entries()].filter(([, count]) => count > 1).map(([key]) => key))
}

async function runUnit<T>(unit: ParallelUnit<T>, exclusive: boolean): Promise<ParallelUnitResult<T>> {
  try {
    return { id: unit.id, status: 'ok', exclusive, value: await unit.run() }
  } catch (error) {
    return {
      id: unit.id,
      status: 'failed',
      exclusive,
      error: error instanceof Error ? error.message : String(error),
    }
  }
}

/**
 * 受控并行执行。独占单元是**组边界（屏障）**：它之前累积的私有单元先跑完，它单独跑，
 * 之后才继续；私有单元按 `concurrency` 切片成组并发。任一组出现失败即停止派发，
 * 其后的单元全部记 `skipped`（显式未完成）。
 */
export async function runControlledParallel<T>(
  units: readonly ParallelUnit<T>[],
  options: { readonly concurrency?: number; readonly sharedWriteSurfaces?: readonly string[] } = {},
): Promise<ParallelOutcome<T>> {
  const seen = new Set<string>()
  for (const unit of units) {
    if (unit.id.trim() === '') throw new Error('受控并行：单元 id 必填')
    if (seen.has(unit.id)) throw new Error(`受控并行：单元 id 重复 ${JSON.stringify(unit.id)}`)
    seen.add(unit.id)
  }
  const concurrency = Math.max(1, Math.floor(options.concurrency ?? 1))
  const surfaces = options.sharedWriteSurfaces ?? []
  const duplicated = duplicatedWrites(units)
  const exclusive = units.map(unit =>
    unit.writes.length === 0
    || unit.writes.some(write => surfaces.some(surface => pathTouchesSurface(write, surface)))
    || unit.writes.some(write => duplicated.has(normalizeWrite(write))),
  )

  const results = Array.from<ParallelUnitResult<T> | undefined>({ length: units.length })
  let failed = false
  let index = 0
  while (index < units.length && !failed) {
    if (exclusive[index] === true) {
      const result = await runUnit(units[index] as ParallelUnit<T>, true)
      results[index] = result
      failed = result.status === 'failed'
      index += 1
      continue
    }
    const group: number[] = []
    let cursor = index
    while (cursor < units.length && exclusive[cursor] !== true && group.length < concurrency) {
      group.push(cursor)
      cursor += 1
    }
    const settled = await Promise.all(group.map(async position => runUnit(units[position] as ParallelUnit<T>, false)))
    group.forEach((position, order) => {
      results[position] = settled[order]
    })
    failed = settled.some(result => result.status === 'failed')
    index = cursor
  }

  const ordered = units.map((unit, position) => results[position] ?? {
    id: unit.id,
    status: 'skipped' as const,
    exclusive: exclusive[position] === true,
  })
  const incomplete = ordered.filter(result => result.status !== 'ok').map(result => result.id)
  return {
    ok: incomplete.length === 0,
    results: ordered,
    concurrency,
    incomplete,
    exclusive: units.filter((_, position) => exclusive[position] === true).map(unit => unit.id),
  }
}
