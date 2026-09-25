/**
 * Node/Bun RSS attribution probe — answers "why did that server eat N GiB?".
 *
 * A jetsam/OOM report tells you *which* process died and how large it was; it
 * never records argv and never says what the size was made of. This probe closes
 * that gap **while the process is alive**, because that distinction is the whole
 * answer: `--max-old-space-size` constrains the V8 GC heap only, and the failure
 * we hit was 11.10 GiB RSS against a 4.19 GiB V8 heap limit — i.e. at least
 * ~7 GiB lived outside the GC heap (Buffer / ArrayBuffer / native arena / dirty
 * file-backed pages), where no JS-heap flag and no `--heapsnapshot-near-heap-limit`
 * can see it. Capping would have been a no-op on that failure mode.
 *
 * Every interval it:
 *   1. reads one `ps -Ao pid,ppid,rss,command` for topology and one
 *      `footprint -p …` batch for the charged footprint of every tracked pid
 *   2. appends a JSONL sample (charged + RSS totals + top consumers) and prints
 *      one status line
 *   3. when a tracked pid crosses `--threshold-mb` (charged), captures
 *      `vmmap -summary` and `footprint -p` (read-only) and — only when the target
 *      is known to be instrumented — sends SIGUSR2 so node writes a diagnostic
 *      report whose `javascriptHeap` vs `resourceUsage.maxRSS` split names the
 *      culprit class
 *
 * The charged footprint is the primary metric, not `ps` RSS: macOS reclaims pages
 * by compression, so RSS misses exactly the memory that kills the machine. Both
 * are recorded because the divergence is itself evidence (measured: the dev vite
 * server sat at 36 MiB RSS / 941 MiB footprint, codegraph at 136 MiB / 782 MiB).
 *
 * Instrumentation is what makes the in-process half work; `--launch` injects it
 * into the child's NODE_OPTIONS for you (node accepts `--report-*` there):
 *
 *   pnpm run probe:node-memory                                  # 自动发现全部 node/bun
 *   pnpm run probe:node-memory --port 3100 --interval-sec 10
 *   pnpm run probe:node-memory --launch "mise run launch" --threshold-mb 1500
 *
 * SIGUSR2 is NEVER sent unless we injected the flags (`--launch`) or the operator
 * asserts it (`--instrumented`): a node process without `--report-on-signal`
 * treats SIGUSR2 as terminate, and killing the process under investigation is
 * worse than not measuring it.
 *
 * Finer-grained in-process sampling (external vs arrayBuffers) belongs in the
 * candidate itself — one line, no dependency:
 *   setInterval(() => { const m = process.memoryUsage()
 *     console.log('mem', m.rss, m.heapUsed, m.external, m.arrayBuffers) }, 30_000)
 */
import { execFileSync, spawn } from 'node:child_process'
import type { ChildProcess } from 'node:child_process'
import { appendFileSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from 'node:fs'
import { homedir } from 'node:os'
import path from 'node:path'
import process from 'node:process'

interface Options {
  readonly pids: readonly number[]
  readonly ports: readonly number[]
  readonly launch: string | undefined
  readonly instrumented: boolean
  readonly intervalSec: number
  readonly durationSec: number
  readonly thresholdMb: number
  readonly outDir: string
  readonly reportDir: string
}

interface ProcRow {
  readonly pid: number
  readonly ppid: number
  readonly rssKib: number
  readonly command: string
}

interface Tracked {
  readonly pid: number
  command: string
  firstKib: number
  lastKib: number
  peakKib: number
  /** Charged footprint — the number jetsam acts on; undefined until first sample. */
  firstFootKib: number | undefined
  lastFootKib: number | undefined
  peakFootKib: number
  /** Seen gone from `ps`: freeze the last numbers instead of zeroing them. */
  exited: boolean
  capturedAtMs: number
  report: string | undefined
}

const USAGE = `用法: pnpm run probe:node-memory [选项]

  --pid <n>            跟踪该进程及其后代（可重复）
  --port <n>           先解析监听该端口的 pid 再跟踪（可重复）
  --launch "<cmd>"     以 shell 启动候选，并把 --report-* 注入其 NODE_OPTIONS
  --instrumented       断言目标已带 --report-on-signal，允许发 SIGUSR2
  --interval-sec <n>   采样间隔，默认 15
  --duration-sec <n>   总时长，0 = 直到 Ctrl-C（默认 0）
  --threshold-mb <n>   触发 vmmap/footprint/诊断报告的 RSS 阈值，默认 1500
  --out <dir>          证据目录，默认 ~/.dsh-alioth/memory-probe/<时间戳>
  --report-dir <dir>   node 诊断报告目录，默认 <out>/node-reports

未给 --pid/--port/--launch 时自动发现全部 node/bun 进程（排除探针自身）。`

function usageError(message: string): never {
  process.stderr.write(`node-memory-probe: ${message}\n\n${USAGE}\n`)
  process.exit(2)
}

function parseArgs(argv: readonly string[]): Options {
  const pids: number[] = []
  const ports: number[] = []
  let launch: string | undefined
  let instrumented = false
  let intervalSec = 15
  let durationSec = 0
  let thresholdMb = 1500
  let outDir: string | undefined
  let reportDir: string | undefined

  const number = (flag: string, raw: string | undefined): number => {
    if (raw === undefined) usageError(`${flag} 需要一个数值`)
    const value = Number(raw)
    if (!Number.isFinite(value) || value < 0) usageError(`${flag} 不是合法数值: ${String(raw)}`)
    return value
  }

  for (let index = 0; index < argv.length; index += 1) {
    const flag = argv[index]
    const next = argv[index + 1]
    switch (flag) {
      case '--pid': pids.push(number(flag, next)); index += 1; break
      case '--port': ports.push(number(flag, next)); index += 1; break
      case '--launch':
        if (next === undefined || next.length === 0) usageError('--launch 需要一条命令')
        launch = next; index += 1; break
      case '--instrumented': instrumented = true; break
      case '--interval-sec': intervalSec = number(flag, next); index += 1; break
      case '--duration-sec': durationSec = number(flag, next); index += 1; break
      case '--threshold-mb': thresholdMb = number(flag, next); index += 1; break
      case '--out': outDir = next; index += 1; break
      case '--report-dir': reportDir = next; index += 1; break
      case '--help': case '-h': process.stdout.write(`${USAGE}\n`); process.exit(0); break
      default: usageError(`未知选项 ${String(flag)}`)
    }
  }

  if (intervalSec <= 0) usageError('--interval-sec 必须 > 0')
  const stamp = new Date().toISOString().replace(/[:.]/g, '-')
  const resolvedOut = path.resolve(outDir ?? path.join(homedir(), '.dsh-alioth', 'memory-probe', stamp))
  return {
    pids,
    ports,
    launch,
    instrumented: instrumented || launch !== undefined,
    intervalSec,
    durationSec,
    thresholdMb,
    outDir: resolvedOut,
    reportDir: path.resolve(reportDir ?? path.join(resolvedOut, 'node-reports')),
  }
}

function log(line: string): void {
  process.stdout.write(`node-memory-probe: ${line}\n`)
}

function sleepSync(ms: number): void {
  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms)
}

/** KiB → GiB label used by every console line and by the sample tail. */
function gib(kib: number): string {
  return `${(kib / 1024 / 1024).toFixed(2)} GiB`
}

/**
 * The metric every threshold and verdict uses: charged footprint (what jetsam
 * counts) falling back to RSS only when `footprint(1)` is unavailable.
 */
function chargedKib(item: Tracked): number {
  return item.lastFootKib ?? item.lastKib
}

/** One `ps` call per sample: pid/ppid/rss are machine-stable fields of `-o`. */
function listProcesses(): ProcRow[] {
  const output = execFileSync('ps', ['-Ao', 'pid,ppid,rss,command'], { encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 })
  const rows: ProcRow[] = []
  for (const line of output.split('\n').slice(1)) {
    const parts = line.trim().split(/\s+/)
    if (parts.length < 4) continue
    const pid = Number(parts[0])
    const ppid = Number(parts[1])
    const rssKib = Number(parts[2])
    if (!Number.isFinite(pid) || !Number.isFinite(ppid) || !Number.isFinite(rssKib)) continue
    rows.push({ pid, ppid, rssKib, command: parts.slice(3).join(' ') })
  }
  return rows
}

function isRuntime(command: string): boolean {
  const first = command.split(' ')[0] ?? ''
  const base = path.basename(first)
  return base === 'node' || base === 'bun'
}

const FOOTPRINT_LINE = /^\S.*\[(\d+)\]:.*?Footprint:\s+([\d.]+)\s+(bytes|KB|MB|GB)/

/**
 * One `footprint -p a -p b …` call (~0.2 s for 8 pids, measured) reports the
 * charged footprint of every pid. All pids must be alive: a dead one makes the
 * whole call fail, which would blind the sample.
 */
function chargedFootprintsKib(pids: readonly number[]): Map<number, number> {
  const result = new Map<number, number>()
  if (pids.length === 0) return result
  try {
    const output = execFileSync('footprint', pids.flatMap((pid) => ['-p', String(pid)]), {
      encoding: 'utf8',
      maxBuffer: 256 * 1024 * 1024,
    })
    for (const line of output.split('\n')) {
      const match = FOOTPRINT_LINE.exec(line)
      if (match === null) continue
      const pid = Number(match[1])
      const value = Number(match[2])
      const unit = match[3] ?? ''
      if (!Number.isFinite(pid) || !Number.isFinite(value)) continue
      const kib = unit === 'bytes' ? value / 1024
        : unit === 'KB' ? value
          : unit === 'MB' ? value * 1024
            : value * 1024 * 1024
      result.set(pid, kib)
    }
  } catch (error) {
    log(`footprint 不可用 (${String(error)}) —— 回退到 ps RSS，注意 RSS 会漏掉被压缩回收的页`)
  }
  return result
}

function descendantPids(rows: readonly ProcRow[], roots: readonly number[]): Set<number> {
  const children = new Map<number, number[]>()
  for (const row of rows) {
    const list = children.get(row.ppid)
    if (list === undefined) children.set(row.ppid, [row.pid])
    else list.push(row.pid)
  }
  const seen = new Set<number>()
  const queue = [...roots]
  while (queue.length > 0) {
    const pid = queue.pop()
    if (pid === undefined || seen.has(pid)) continue
    seen.add(pid)
    for (const child of children.get(pid) ?? []) queue.push(child)
  }
  return seen
}

function listenerPids(port: number): number[] {
  const output = execFileSync('lsof', ['-nP', `-tiTCP:${port}`, '-sTCP:LISTEN'], { encoding: 'utf8' })
  return output.split('\n').map((line) => Number(line.trim())).filter((pid) => Number.isFinite(pid) && pid > 0)
}

function capture(kind: 'vmmap' | 'footprint', pid: number, options: Options, stamp: string): string | undefined {
  const file = path.join(options.outDir, `${kind}-${pid}-${stamp}.txt`)
  const args = kind === 'vmmap' ? ['-summary', String(pid)] : ['-p', String(pid)]
  try {
    writeFileSync(file, execFileSync(kind, args, { encoding: 'utf8', maxBuffer: 256 * 1024 * 1024 }))
    return file
  } catch (error) {
    log(`无法执行 ${kind} pid=${pid}: ${String(error)}`)
    return undefined
  }
}

interface NodeReport {
  readonly header?: { readonly event?: string; readonly trigger?: string; readonly commandLine?: readonly string[] }
  readonly javascriptHeap?: {
    /** JS heap in use — the only part `--max-old-space-size` constrains. */
    readonly usedMemory?: number
    readonly memoryLimit?: number
    /** ArrayBuffer/Buffer backing stores: where an off-heap balloon shows up. */
    readonly externalMemory?: number
    /** V8's own native allocations. */
    readonly mallocedMemory?: number
    readonly peakMallocedMemory?: number
  }
  readonly resourceUsage?: {
    readonly rss?: number
    readonly maxRss?: number
    readonly available_memory?: number
    readonly total_memory?: number
  }
}

/**
 * Ask an already-instrumented node for its diagnostic report. Returns the report
 * path, or undefined when nothing appeared (the process was not instrumented —
 * which is why the caller must never send SIGUSR2 on a guess).
 */
function requestNodeReport(pid: number, options: Options): string | undefined {
  const before = new Set(readdirSync(options.reportDir))
  try {
    process.kill(pid, 'SIGUSR2')
  } catch (error) {
    log(`无法向 pid=${pid} 发 SIGUSR2: ${String(error)}`)
    return undefined
  }
  const deadline = Date.now() + 10_000
  while (Date.now() < deadline) {
    const fresh = readdirSync(options.reportDir)
      .filter((name) => name.endsWith('.json') && !before.has(name))
      .sort()
    const newest = fresh.at(-1)
    if (newest !== undefined) return path.join(options.reportDir, newest)
    sleepSync(200)
  }
  return undefined
}

/** Byte counts in a diagnostic report — same unit family as the V8 heap limit. */
function mib(bytes: number | undefined): string {
  return bytes === undefined ? '?' : `${(bytes / 2 ** 20).toFixed(1)} MiB`
}

/**
 * One line that answers the question: the JS heap is `usedMemory` against
 * `memoryLimit` (~4.19 GiB on a 24 GiB box), everything else that survives is
 * `externalMemory` (ArrayBuffers/Buffers) or `mallocedMemory` (native) — so a
 * balloon at 11 GiB RSS with a small JS heap is off-heap by construction, and
 * reporting the share makes that explicit instead of leaving it to arithmetic.
 */
function reportBreakdown(file: string): string {
  const report = JSON.parse(readFileSync(file, 'utf8')) as NodeReport
  const heap = report.javascriptHeap ?? {}
  const usage = report.resourceUsage ?? {}
  const offHeap = (heap.externalMemory ?? 0) + (heap.mallocedMemory ?? 0)
  const share = usage.maxRss === undefined || usage.maxRss === 0
    ? '?'
    : `${((offHeap / usage.maxRss) * 100).toFixed(0)}%`
  const argv = report.header?.commandLine?.join(' ')
  return `V8 堆 used ${mib(heap.usedMemory)} / limit ${heap.memoryLimit === undefined ? '?' : `${(heap.memoryLimit / 2 ** 30).toFixed(2)} GiB`}`
    + ` · external ${mib(heap.externalMemory)} · malloced ${mib(heap.mallocedMemory)}`
    + ` · rss ${mib(usage.rss)} / maxRss ${mib(usage.maxRss)} ⇒ 堆外(external+malloced) 占 maxRss ${share}`
    + (argv === undefined ? '' : ` · cmd ${argv}`)
}

function instrumentedLaunch(command: string, options: Options): ChildProcess {
  mkdirSync(options.reportDir, { recursive: true })
  const injected = '--report-on-signal --report-signal=SIGUSR2 --report-directory=' + options.reportDir
  const existing = process.env['NODE_OPTIONS'] ?? ''
  const nodeOptions = existing.includes('--report-on-signal') ? existing : `${existing} ${injected}`.trim()
  return spawn(command, {
    shell: true,
    stdio: 'inherit',
    env: { ...process.env, NODE_OPTIONS: nodeOptions },
  })
}

function main(): void {
  const options = parseArgs(process.argv.slice(2))
  mkdirSync(options.outDir, { recursive: true })
  mkdirSync(options.reportDir, { recursive: true })
  const samplesFile = path.join(options.outDir, 'samples.jsonl')

  const roots: number[] = [...options.pids]
  for (const port of options.ports) {
    const pids = listenerPids(port)
    if (pids.length === 0) usageError(`端口 ${port} 上没有监听进程`)
    roots.push(...pids)
  }

  let launched: ChildProcess | undefined
  if (options.launch !== undefined) {
    launched = instrumentedLaunch(options.launch, options)
    if (launched.pid !== undefined) roots.push(launched.pid)
    log(`已启动候选: ${options.launch} (pid ${String(launched.pid)})，诊断报告目录 ${options.reportDir}`)
  }

  const autoDiscover = roots.length === 0
  log(`out=${options.outDir} 阈值=${options.thresholdMb} MiB 间隔=${options.intervalSec}s `
    + `时长=${options.durationSec === 0 ? 'Ctrl-C 结束' : `${options.durationSec}s`} `
    + `模式=${autoDiscover ? '自动发现 node/bun' : `跟踪 ${roots.join(',')} 及其后代`}`)
  if (options.instrumented && autoDiscover) {
    log('注意: 自动发现模式下我们不注入 NODE_OPTIONS —— 未知目标一律不发 SIGUSR2（未 instrumented 的 node 会把 SIGUSR2 当终止信号）')
  }

  const tracked = new Map<number, Tracked>()
  let stopping = false
  process.on('SIGINT', () => { stopping = true })
  process.on('SIGTERM', () => { stopping = true })

  const startedAt = Date.now()
  const deadline = options.durationSec === 0 ? Number.POSITIVE_INFINITY : startedAt + options.durationSec * 1000
  let rounds = 0

  while (!stopping && Date.now() < deadline) {
    if (launched !== undefined && launched.exitCode !== null) {
      log(`候选进程已退出 (code ${String(launched.exitCode)})，停止采样`)
      break
    }
    rounds += 1
    const rows = listProcesses()
    const inTree = autoDiscover ? undefined : descendantPids(rows, roots)
    const selected = (autoDiscover
      ? rows.filter((row) => isRuntime(row.command) && !row.command.includes('node-memory-probe'))
      : rows.filter((row) => inTree !== undefined && inTree.has(row.pid)))
      .filter((row) => !row.command.includes('<defunct>'))
    const footprints = chargedFootprintsKib(selected.map((row) => row.pid).slice(0, 128))

    for (const row of selected) {
      const footKib = footprints.get(row.pid)
      const existing = tracked.get(row.pid)
      if (existing === undefined) {
        tracked.set(row.pid, {
          pid: row.pid,
          command: row.command,
          firstKib: row.rssKib,
          lastKib: row.rssKib,
          peakKib: row.rssKib,
          firstFootKib: footKib,
          lastFootKib: footKib,
          peakFootKib: footKib ?? 0,
          exited: false,
          capturedAtMs: 0,
          report: undefined,
        })
        continue
      }
      if (existing.firstFootKib === undefined) existing.firstFootKib = footKib
      existing.lastKib = row.rssKib
      existing.peakKib = Math.max(existing.peakKib, row.rssKib)
      existing.command = row.command
      existing.exited = false
      if (footKib !== undefined) {
        existing.lastFootKib = footKib
        existing.peakFootKib = Math.max(existing.peakFootKib, footKib)
      }
    }
    const alive = new Set(selected.map((row) => row.pid))
    for (const item of tracked.values()) if (!alive.has(item.pid)) item.exited = true

    const values = [...tracked.values()]
    const top = [...values].sort((a, b) => chargedKib(b) - chargedKib(a)).slice(0, 8)
    const totalKib = values.reduce((sum, item) => sum + chargedKib(item), 0)
    const totalRssKib = values.reduce((sum, item) => sum + item.lastKib, 0)
    appendFileSync(samplesFile, `${JSON.stringify({
      t: new Date().toISOString(),
      round: rounds,
      trackedCount: tracked.size,
      chargedTotalMiB: Math.round(totalKib / 1024),
      rssTotalMiB: Math.round(totalRssKib / 1024),
      top: top.map((item) => ({
        pid: item.pid,
        chargedMiB: Math.round(chargedKib(item) / 1024),
        rssMiB: Math.round(item.lastKib / 1024),
        cmd: item.command.slice(0, 160),
      })),
    })}\n`)
    const head = top[0]
    log(`#${rounds} 跟踪 ${tracked.size} 个 · charged 合计 ${gib(totalKib)} (RSS ${gib(totalRssKib)})`
      + (head === undefined ? '' : ` · 最大 pid ${head.pid} ${gib(chargedKib(head))}`))

    const stamp = new Date().toISOString().replace(/[:.]/g, '-')
    for (const item of tracked.values()) {
      if (item.exited) continue
      const metricKib = chargedKib(item)
      const overThreshold = metricKib / 1024 > options.thresholdMb
      const rearms = Date.now() - item.capturedAtMs > 300_000
      if (!overThreshold || !rearms) continue
      item.capturedAtMs = Date.now()
      log(`pid ${item.pid} 超阈值 (${
        item.lastFootKib === undefined ? 'RSS' : 'charged'} ${gib(metricKib)} > ${options.thresholdMb} MiB`
        + `, RSS ${gib(item.lastKib)}): ${item.command.slice(0, 160)}`)
      for (const kind of ['vmmap', 'footprint'] as const) {
        const file = capture(kind, item.pid, options, stamp)
        if (file !== undefined) log(`  ${kind} → ${file}`)
      }
      if (!options.instrumented) {
        log('  未声明 instrumented，跳过 SIGUSR2（未 instrumented 的 node 会把 SIGUSR2 当终止信号）；'
          + '要拿堆内/堆外拆分，用 --launch 或给候选加 NODE_OPTIONS="--report-on-signal --report-signal=SIGUSR2 --report-directory=…"')
        continue
      }
      const report = requestNodeReport(item.pid, options)
      if (report === undefined) {
        log('  10s 内没有新诊断报告：该进程并未带 --report-on-signal（或报告目录不对）')
        continue
      }
      item.report = report
      log(`  诊断报告 ${report}`)
      log(`  ${reportBreakdown(report)}`)
    }

    sleepSync(Math.min(options.intervalSec * 1000, Math.max(0, deadline - Date.now())))
  }

  if (launched !== undefined && launched.exitCode === null) launched.kill('SIGTERM')

  const summary = [...tracked.values()]
    .sort((a, b) => (b.peakFootKib - (b.firstFootKib ?? b.firstKib)) - (a.peakFootKib - (a.firstFootKib ?? a.firstKib)))
    .map((item) => ({
      pid: item.pid,
      command: item.command,
      exited: item.exited,
      rssFirstMiB: Math.round(item.firstKib / 1024),
      rssLastMiB: Math.round(item.lastKib / 1024),
      rssPeakMiB: Math.round(item.peakKib / 1024),
      chargedFirstMiB: item.firstFootKib === undefined ? null : Math.round(item.firstFootKib / 1024),
      chargedLastMiB: item.lastFootKib === undefined ? null : Math.round(item.lastFootKib / 1024),
      chargedPeakMiB: item.peakFootKib === 0 ? null : Math.round(item.peakFootKib / 1024),
      report: item.report,
    }))
  writeFileSync(path.join(options.outDir, 'summary.json'), `${JSON.stringify({
    startedAt: new Date(startedAt).toISOString(),
    intervalSec: options.intervalSec,
    rounds,
    thresholdMiB: options.thresholdMb,
    tracked: summary,
  }, null, 2)}\n`)

  log(`采样 ${rounds} 轮结束，证据: ${options.outDir}（samples.jsonl / summary.json / vmmap-*.txt / footprint-*.txt）`)
  for (const item of summary.slice(0, 10)) {
    const chargedTail = item.chargedFirstMiB === null || item.chargedLastMiB === null
      ? 'charged 不可用'
      : `charged ${String(item.chargedFirstMiB).padStart(6)} → ${String(item.chargedLastMiB).padStart(6)} MiB (峰 ${item.chargedPeakMiB})`
    log(`  pid ${item.pid} ${chargedTail} · RSS ${String(item.rssFirstMiB).padStart(6)} → ${String(item.rssLastMiB).padStart(6)} MiB`
      + `${item.exited ? ' [已退出]' : ''} ${item.command.slice(0, 110)}`)
  }
  const biggest = summary[0]
  if (biggest !== undefined && biggest.report !== undefined) {
    log(`  堆内/堆外拆分: ${reportBreakdown(biggest.report)}`)
  }
}

try {
  main()
} catch (error) {
  process.stderr.write(`node-memory-probe: ${error instanceof Error ? error.message : String(error)}\n`)
  process.exit(1)
}
