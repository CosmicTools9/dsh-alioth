/**
 * zip-store — the download route's archive writer, proven against a real unzip:
 * every byte that goes in comes back out of `unzip -p` unchanged, and the CRCs
 * are verified by `unzip -t` (Info-ZIP's own CRC-32) rather than by our arithmetic.
 *
 * macOS ships Info-ZIP 6.00 without Unicode support: it ignores general purpose
 * bit 11 and so cannot *address* a non-ASCII entry from the command line — the
 * name argument never matches (rc=11) even though `unzip -t` verifies that same
 * entry. Rather than pin that local quirk, the UTF-8 cases read their bytes back
 * through a whole-archive `unzip -p` (entries are concatenated in order) and the
 * stored name bytes plus the bit-11 flag are asserted structurally — that is the
 * part modern extractors read, including Finder's own Archive Utility.
 *
 * The suite that needs `unzip` is skipped, with the reason in its name, when the
 * binary is not on PATH; the structural contracts are asserted without it.
 */
import { describe, expect, it, beforeAll, afterAll } from 'vitest'
import { spawnSync } from 'node:child_process'
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { zipStore, type ZipEntry } from '../src/zip-store.ts'

/** IEEE 802.3 CRC-32 check value for this string is 0xCBF43926. */
const CRC32_VECTOR = '123456789'

/** Every byte value, to catch anything that mangles 0x00 / 0x1A / high bytes. */
const ALL_BYTES = Uint8Array.from({ length: 256 }, (_unused, index) => index)

const UTF8 = new TextEncoder()

let workDir: string

beforeAll(async () => {
  workDir = await mkdtemp(path.join(tmpdir(), 'dsh-alioth-zip-store-'))
})

afterAll(async () => {
  await rm(workDir, { recursive: true, force: true })
})

/** Pack `entries` and drop the archive where `unzip` can read it. */
async function writeArchive(name: string, entries: readonly ZipEntry[]): Promise<string> {
  const file = path.join(workDir, name)
  await writeFile(file, zipStore(entries))
  return file
}

/** One archive's central directory entry, with the fields of its local header. */
interface ArchiveEntry {
  readonly name: string
  readonly nameBytes: Uint8Array
  readonly centralFlags: number
  readonly localFlags: number
  readonly method: number
  readonly crc: number
  readonly compressedSize: number
  readonly uncompressedSize: number
  /** Where this entry's stored payload starts in the archive. */
  readonly dataOffset: number
}

interface Archive {
  readonly entries: readonly ArchiveEntry[]
  readonly count: number
  readonly centralOffset: number
  readonly centralSize: number
}

/** Read back an archive the way an extractor does: through the central directory. */
function readArchive(archive: Uint8Array): Archive {
  const view = new DataView(archive.buffer, archive.byteOffset, archive.byteLength)
  const end = archive.length - 22
  if (archive.length < 22 || view.getUint32(end, true) !== 0x06054b50) {
    throw new Error('archive has no end-of-central-directory record')
  }
  const count = view.getUint16(end + 10, true)
  const centralSize = view.getUint32(end + 12, true)
  const centralOffset = view.getUint32(end + 16, true)
  const entries: ArchiveEntry[] = []
  let cursor = centralOffset
  for (let index = 0; index < count; index += 1) {
    if (view.getUint32(cursor, true) !== 0x02014b50) {
      throw new Error(`central directory entry ${index} does not start with its signature`)
    }
    const nameLength = view.getUint16(cursor + 28, true)
    const extraLength = view.getUint16(cursor + 30, true)
    const commentLength = view.getUint16(cursor + 32, true)
    const localOffset = view.getUint32(cursor + 42, true)
    const nameBytes = archive.subarray(cursor + 46, cursor + 46 + nameLength)
    if (view.getUint32(localOffset, true) !== 0x04034b50) {
      throw new Error(`entry '${new TextDecoder().decode(nameBytes)}' has no local file header`)
    }
    const localNameLength = view.getUint16(localOffset + 26, true)
    const localExtraLength = view.getUint16(localOffset + 28, true)
    entries.push({
      name: new TextDecoder().decode(nameBytes),
      nameBytes,
      centralFlags: view.getUint16(cursor + 8, true),
      localFlags: view.getUint16(localOffset + 6, true),
      method: view.getUint16(cursor + 10, true),
      crc: view.getUint32(cursor + 16, true),
      compressedSize: view.getUint32(cursor + 20, true),
      uncompressedSize: view.getUint32(cursor + 24, true),
      dataOffset: localOffset + 30 + localNameLength + localExtraLength,
    })
    cursor += 46 + nameLength + extraLength + commentLength
  }
  return { entries, count, centralOffset, centralSize }
}

const unzipAvailable = spawnSync('unzip', ['-v']).error === undefined

interface UnzipResult {
  readonly status: number
  readonly stdout: Uint8Array
  readonly stderr: string
}

/** Run the system `unzip`; stdout stays raw bytes so payloads can be compared. */
function unzip(args: readonly string[]): UnzipResult {
  const result = spawnSync('unzip', args, { maxBuffer: 64 * 1024 * 1024 })
  if (result.error) throw result.error
  return {
    status: result.status ?? -1,
    stdout: new Uint8Array(result.stdout.buffer, result.stdout.byteOffset, result.stdout.byteLength),
    stderr: result.stderr.toString('utf8'),
  }
}

describe.skipIf(!unzipAvailable)('zip-store round trip through the system `unzip` [requires `unzip` on PATH]', () => {
  it('passes `unzip -t` and returns every entry byte for byte from `unzip -p`', async () => {
    const entries: readonly ZipEntry[] = [
      { name: 'readme.txt', data: UTF8.encode('hello from the paid download\n'), mtime: new Date(2024, 4, 6, 7, 8, 10) },
      { name: 'src/deep/nested.ts', data: UTF8.encode('export const answer = 42\n') },
      { name: 'empty.bin', data: new Uint8Array(0) },
      { name: 'bytes/all.bin', data: ALL_BYTES },
    ]
    const file = await writeArchive('roundtrip.zip', entries)

    const tested = unzip(['-t', file])
    expect(tested.status, new TextDecoder().decode(tested.stdout)).toBe(0)

    for (const entry of entries) {
      const extracted = unzip(['-p', file, entry.name])
      expect(extracted.status, `${entry.name}: ${extracted.stderr}`).toBe(0)
      expect(extracted.stdout, entry.name).toEqual(entry.data)
    }
  })

  it('carries UTF-8 names whose bytes come back unchanged through the archive dump', async () => {
    const entries: readonly ZipEntry[] = [
      { name: '文档/说明.txt', data: UTF8.encode('中文内容') },
      { name: 'readme.txt', data: UTF8.encode('ascii') },
      { name: 'café/résumé 🚀.txt', data: ALL_BYTES },
      { name: 'empty-文档.bin', data: new Uint8Array(0) },
    ]
    const file = await writeArchive('utf8-names.zip', entries)

    // Info-ZIP verifies the CRC of every entry, non-ASCII names included.
    const tested = unzip(['-t', file])
    expect(tested.status, new TextDecoder().decode(tested.stdout)).toBe(0)

    // `unzip -p` without a name filter concatenates every entry, in archive order.
    const dumped = unzip(['-p', file])
    expect(dumped.status, dumped.stderr).toBe(0)
    let cursor = 0
    for (const entry of entries) {
      expect(dumped.stdout.subarray(cursor, cursor + entry.data.length), entry.name).toEqual(entry.data)
      cursor += entry.data.length
    }
    expect(cursor, 'the dump holds exactly the packed payloads').toBe(dumped.stdout.length)
  })

  it('round-trips a payload past the 16-bit range, where sizes and offsets are 32-bit', async () => {
    // A deterministic 100 KiB body: long enough that the entry after it starts
    // beyond 0xFFFF, so a 16-bit size or offset field would show up as a failure.
    const large = Uint8Array.from({ length: 100_000 }, (_unused, index) => (index * 31) & 0xff)
    const entries: readonly ZipEntry[] = [
      { name: 'large.bin', data: large },
      { name: 'after-large.txt', data: UTF8.encode('past the 16-bit boundary\n') },
    ]
    const file = await writeArchive('large.zip', entries)

    const tested = unzip(['-t', file])
    expect(tested.status, new TextDecoder().decode(tested.stdout)).toBe(0)

    const extracted = unzip(['-p', file, 'after-large.txt'])
    expect(extracted.status, extracted.stderr).toBe(0)
    expect(extracted.stdout).toEqual(entries[1]?.data)

    const parsed = readArchive(zipStore(entries))
    expect(parsed.entries[1]?.dataOffset).toBeGreaterThan(0xffff)
    expect(parsed.entries[0]?.uncompressedSize).toBe(large.length)
  })

  it('encodes the DOS timestamp from `mtime`', async () => {
    const mtime = new Date(2024, 4, 6, 7, 8, 10)
    const file = await writeArchive('stamped.zip', [
      { name: 'stamped.txt', data: UTF8.encode('stamped'), mtime },
    ])

    // The encoded pair is the contract; DOS packs LOCAL date/time into its own
    // fields, and `unzip -l` renders them per platform and version (a printed
    // string assertion is a timezone trap, not a zip guarantee).
    const stamp = readDosStamp(await readFile(file))
    expect(stamp).toEqual({
      date: ((mtime.getFullYear() - 1980) << 9) | ((mtime.getMonth() + 1) << 5) | mtime.getDate(),
      time: (mtime.getHours() << 11) | (mtime.getMinutes() << 5) | (mtime.getSeconds() >> 1),
    })

    // …and the system unzip still recognises the entry it stamped.
    const listed = unzip(['-l', file])
    expect(listed.status, listed.stderr).toBe(0)
    expect(new TextDecoder().decode(listed.stdout)).toContain('stamped.txt')
  })
})

describe('zip-store archive structure', () => {
  it('stores the payload verbatim under a UTF-8 flagged name, in call order', () => {
    const entries: readonly ZipEntry[] = [
      { name: 'readme.txt', data: UTF8.encode(CRC32_VECTOR) },
      { name: '文档/说明.txt', data: UTF8.encode('中文内容') },
      { name: 'nested/deep/empty.bin', data: new Uint8Array(0) },
      { name: 'bytes/all.bin', data: ALL_BYTES },
    ]
    const archive = zipStore(entries)
    const parsed = readArchive(archive)

    expect(parsed.entries.map(entry => entry.name)).toEqual(entries.map(entry => entry.name))
    for (const [index, entry] of parsed.entries.entries()) {
      const source = entries[index] as ZipEntry
      expect(entry.centralFlags & 0x0800, entry.name).toBe(0x0800)
      expect(entry.localFlags & 0x0800, entry.name).toBe(0x0800)
      expect(entry.method, entry.name).toBe(0)
      expect(entry.compressedSize, entry.name).toBe(source.data.length)
      expect(entry.uncompressedSize, entry.name).toBe(source.data.length)
      expect(Array.from(entry.nameBytes), entry.name).toEqual(Array.from(UTF8.encode(source.name)))
      expect(Array.from(archive.subarray(entry.dataOffset, entry.dataOffset + source.data.length)), entry.name)
        .toEqual(Array.from(source.data))
    }

    // The CRC-32 field is the IEEE 802.3 check value, not just self-consistent.
    expect(parsed.entries[0]?.crc).toBe(0xcbf43926)
    expect(parsed.count).toBe(entries.length)
    expect(parsed.centralOffset + parsed.centralSize).toBe(archive.length - 22)
  })

  it('stamps the DOS epoch when `mtime` is missing and clamps dates before 1980', () => {
    const { date, time } = readDosStamp(zipStore([{ name: 'a.txt', data: new Uint8Array(0) }]))
    expect({ date, time }).toEqual({ date: 0x0021, time: 0 })

    const clamped = readDosStamp(zipStore([{ name: 'a.txt', data: new Uint8Array(0), mtime: new Date(1970, 0, 1) }]))
    expect(clamped).toEqual({ date: 0x0021, time: 0 })
  })

  it('produces identical bytes for identical input', () => {
    const entries: readonly ZipEntry[] = [
      { name: 'readme.txt', data: UTF8.encode('same') },
      { name: 'bytes/all.bin', data: ALL_BYTES },
    ]
    expect(zipStore(entries)).toEqual(zipStore(entries))
    expect(zipStore(entries)).toEqual(zipStore(entries.map(entry => ({ ...entry }))))
  })

  it('returns a legal, comment-free empty archive for no entries', () => {
    const expected = new Uint8Array(22)
    new DataView(expected.buffer).setUint32(0, 0x06054b50, true)
    expect(zipStore([])).toEqual(expected)
  })

  it('refuses names that escape the archive root or denote a directory', () => {
    const rejected: readonly (readonly [string, RegExp])[] = [
      ['', /must not be empty/],
      ['/etc/passwd', /without a leading '\//],
      ['../outside.txt', /'\.' or '\.\.' segments/],
      ['nested/../../outside.txt', /'\.' or '\.\.' segments/],
      ['..', /'\.' or '\.\.' segments/],
      ['windows\\path.txt', /backslash/],
      ['directory/', /empty path segments/],
      ['double//slash.txt', /empty path segments/],
    ]
    for (const [name, message] of rejected) {
      expect(() => zipStore([{ name, data: new Uint8Array(0) }]), JSON.stringify(name)).toThrow(message)
    }
  })

  it('refuses input that would overflow a zip32 field instead of writing a broken archive', () => {
    expect(() => zipStore([{ name: 'huge.bin', data: withDeclaredLength(0x1_0000_0000) }]))
      .toThrow(/beyond the zip32 limit/)
    expect(() => zipStore([{ name: 'n'.repeat(40), data: withDeclaredLength(0xffff_fff0) }]))
      .toThrow(/beyond the zip32 limit/)
    expect(() => zipStore([{ name: 'a'.repeat(0x10000), data: new Uint8Array(0) }]))
      .toThrow(/beyond the 65535 byte zip32 limit/)
    expect(() => zipStore(Array.from({ length: 0x10000 }, (_unused, index) => ({ name: `f${index}.txt`, data: new Uint8Array(0) }))))
      .toThrow(/exceed the zip32 limit of 65535/)
  })

  it('refuses an mtime that is not a real date', () => {
    expect(() => zipStore([{ name: 'a.txt', data: new Uint8Array(0), mtime: new Date(Number.NaN) }]))
      .toThrow(/invalid mtime/)
  })
})

/** A one-byte buffer that claims a bigger `length`, to reach the size guards. */
function withDeclaredLength(length: number): Uint8Array {
  const data = new Uint8Array(1)
  Object.defineProperty(data, 'length', { value: length })
  return data
}

/** The DOS date/time pair of the first entry's local header. */
function readDosStamp(archive: Uint8Array): { readonly time: number; readonly date: number } {
  const view = new DataView(archive.buffer, archive.byteOffset, archive.byteLength)
  return { time: view.getUint16(10, true), date: view.getUint16(12, true) }
}
