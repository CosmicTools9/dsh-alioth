/**
 * Store-only ZIP writer for the console's paid source-download route: memory
 * files in, one archive out — no compression, no dependencies.
 *
 * `store` leaves the payload untouched, so the archive is a byte copy of what the
 * caller already holds; the route can hand it straight to an HTTP response and the
 * CRC-32 in the headers is the only claim made about the bytes.
 *
 * Two properties make the output trustworthy:
 * - Determinism. No timestamps are invented: an entry without `mtime` is stamped
 *   with the DOS epoch (1980-01-01 00:00), so identical input yields identical
 *   bytes and a route can hand out stable ETags.
 * - Refusal over corruption. Every zip32 field that would otherwise silently
 *   truncate (name length, entry count, 32-bit sizes, offsets) throws instead of
 *   emitting an archive that only fails at the user's unzip.
 *
 * Names are written as UTF-8 with general purpose bit 11 set, which is the truth
 * modern extractors read: macOS Archive Utility/`ditto` (Finder's own path),
 * libarchive, Python's `zipfile`, 7-Zip and Windows Explorer. One caveat worth
 * knowing before changing this: Apple's bundled Info-ZIP `unzip` 6.00 has no
 * Unicode support and ignores bit 11, so it cannot *address* a non-ASCII name
 * from the command line (`unzip -t` still verifies the whole archive, and ASCII
 * names round-trip normally). Finder extraction does not go through that binary.
 * @module @dsh-alioth/auth-web-alioth/zip-store
 */

/** One file to pack. */
export interface ZipEntry {
  /** Archive path, forward-slash separated, relative (no leading '/', no '..'). */
  readonly name: string
  readonly data: Uint8Array
  /** Source mtime; absent → a fixed epoch (deterministic archives). */
  readonly mtime?: Date
}

/** Local file header signature (`PK\x03\x04`, APPNOTE 4.3.7). */
const LOCAL_SIGNATURE = 0x04034b50
/** Central directory header signature (`PK\x01\x02`, APPNOTE 4.3.12). */
const CENTRAL_SIGNATURE = 0x02014b50
/** End of central directory signature (`PK\x05\x06`, APPNOTE 4.3.16). */
const END_SIGNATURE = 0x06054b50

/** Fixed byte cost of each record, before the variable name/data parts. */
const LOCAL_HEADER_BYTES = 30
const CENTRAL_HEADER_BYTES = 46
const END_RECORD_BYTES = 22

/** "Version needed to extract" and the low half of "version made by": 2.0. */
const ZIP_VERSION = 20
/** General purpose bit 11: the file name is UTF-8 (APPNOTE 4.4.4). */
const FLAG_UTF8_NAME = 0x0800
/** Compression method 0: the archive stores data verbatim (APPNOTE 4.4.5). */
const METHOD_STORE = 0

/** Every size, offset and count field here is 32-bit, except the entry count. */
const ZIP32_MAX = 0xffffffff
const MAX_ENTRIES = 0xffff
const MAX_NAME_BYTES = 0xffff

const UTF8 = new TextEncoder()

/**
 * DOS timestamps start at 1980-01-01 00:00 and pack the year into 7 bits, so
 * 2107 is the last representable one. Anything outside is clamped — a date is
 * metadata, and clamping keeps the archive legal where throwing would fail a
 * download over a clock quirk.
 */
const DOS_EPOCH = new Date(1980, 0, 1, 0, 0, 0)
const DOS_LAST_YEAR = 2107

const CRC32_TABLE = buildCrc32Table()

/** Reflected IEEE 802.3 polynomial (APPNOTE 4.4.7). */
function buildCrc32Table(): Uint32Array {
  const table = new Uint32Array(256)
  for (let index = 0; index < 256; index += 1) {
    let value = index
    for (let bit = 0; bit < 8; bit += 1) {
      value = (value & 1) === 1 ? (0xedb88320 ^ (value >>> 1)) >>> 0 : value >>> 1
    }
    table[index] = value >>> 0
  }
  return table
}

function crc32(data: Uint8Array): number {
  let crc = 0xffffffff
  for (let index = 0; index < data.length; index += 1) {
    crc = (CRC32_TABLE[(crc ^ (data[index] as number)) & 0xff] as number) ^ (crc >>> 8)
  }
  return (crc ^ 0xffffffff) >>> 0
}

/** The 16-bit date/time pair as stored twice per entry (APPNOTE 4.4.6). */
interface DosTimestamp {
  readonly time: number
  readonly date: number
}

function dosTimestamp(name: string, mtime: Date | undefined): DosTimestamp {
  if (mtime !== undefined && Number.isNaN(mtime.getTime())) {
    throw new Error(`zip-store: entry '${name}' has an invalid mtime`)
  }
  const stamp = mtime === undefined || mtime.getTime() < DOS_EPOCH.getTime() ? DOS_EPOCH : mtime
  const year = Math.min(Math.max(stamp.getFullYear(), DOS_EPOCH.getFullYear()), DOS_LAST_YEAR)
  const date = ((year - DOS_EPOCH.getFullYear()) << 9) | ((stamp.getMonth() + 1) << 5) | stamp.getDate()
  const time = (stamp.getHours() << 11) | (stamp.getMinutes() << 5) | (stamp.getSeconds() >> 1)
  return { date: date & 0xffff, time: time & 0xffff }
}

/**
 * Validate an archive path and return its UTF-8 bytes. A name that escapes the
 * archive root (absolute, `..`, backslash-separated) or denotes a directory is a
 * caller bug: writing it would produce an entry that extracts somewhere else, or
 * a directory record this writer never intends to create.
 */
function encodeName(name: string): Uint8Array {
  if (name.length === 0) throw new Error('zip-store: entry name must not be empty')
  if (name.includes('\\')) throw new Error(`zip-store: entry name must not contain a backslash: '${name}'`)
  if (name.startsWith('/')) throw new Error(`zip-store: entry name must be relative, without a leading '/': '${name}'`)
  for (const segment of name.split('/')) {
    if (segment === '') {
      throw new Error(`zip-store: entry name must not contain empty path segments: '${name}'`)
    }
    if (segment === '.' || segment === '..') {
      throw new Error(`zip-store: entry name must not contain '.' or '..' segments: '${name}'`)
    }
  }
  const bytes = UTF8.encode(name)
  if (bytes.length > MAX_NAME_BYTES) {
    throw new Error(`zip-store: entry name needs ${bytes.length} bytes, beyond the ${MAX_NAME_BYTES} byte zip32 limit: '${name.slice(0, 32)}…'`)
  }
  return bytes
}

/** One entry with everything the two header records repeat about it. */
interface PlannedEntry {
  readonly nameBytes: Uint8Array
  readonly data: Uint8Array
  readonly crc: number
  readonly time: number
  readonly date: number
  readonly offset: number
}

interface Layout {
  readonly entries: readonly PlannedEntry[]
  readonly localBytes: number
  readonly centralBytes: number
}

/**
 * Validate every entry and compute the final layout before a single byte is
 * written: all zip32 limits are checked here, so a rejected input never reaches
 * the (potentially huge) output allocation.
 */
function planLayout(entries: readonly ZipEntry[]): Layout {
  if (entries.length > MAX_ENTRIES) {
    throw new Error(`zip-store: ${entries.length} entries exceed the zip32 limit of ${MAX_ENTRIES}`)
  }
  const planned: PlannedEntry[] = []
  let localBytes = 0
  let centralBytes = 0
  for (const entry of entries) {
    const nameBytes = encodeName(entry.name)
    const size = entry.data.length
    if (size > ZIP32_MAX) {
      throw new Error(`zip-store: entry '${entry.name}' holds ${size} bytes, beyond the zip32 limit`)
    }
    const end = localBytes + LOCAL_HEADER_BYTES + nameBytes.length + size
    if (end > ZIP32_MAX) {
      throw new Error(`zip-store: entry '${entry.name}' would end at offset ${end}, beyond the zip32 limit`)
    }
    const { time, date } = dosTimestamp(entry.name, entry.mtime)
    planned.push({ nameBytes, data: entry.data, crc: crc32(entry.data), time, date, offset: localBytes })
    localBytes = end
    centralBytes += CENTRAL_HEADER_BYTES + nameBytes.length
  }
  if (centralBytes > ZIP32_MAX) {
    throw new Error(`zip-store: the central directory needs ${centralBytes} bytes, beyond the zip32 limit`)
  }
  return { entries: planned, localBytes, centralBytes }
}

/** Build a store-only (compression method 0) ZIP archive. Entry order is preserved. */
export function zipStore(entries: readonly ZipEntry[]): Uint8Array {
  const { entries: planned, localBytes, centralBytes } = planLayout(entries)
  const archive = new Uint8Array(localBytes + centralBytes + END_RECORD_BYTES)
  const view = new DataView(archive.buffer, archive.byteOffset, archive.byteLength)
  let cursor = 0
  const u16 = (value: number): void => {
    view.setUint16(cursor, value, true)
    cursor += 2
  }
  const u32 = (value: number): void => {
    view.setUint32(cursor, value, true)
    cursor += 4
  }
  const copy = (value: Uint8Array): void => {
    archive.set(value, cursor)
    cursor += value.length
  }

  // Local file headers, each followed by its stored data.
  for (const entry of planned) {
    const size = entry.data.length
    u32(LOCAL_SIGNATURE)
    u16(ZIP_VERSION)
    u16(FLAG_UTF8_NAME)
    u16(METHOD_STORE)
    u16(entry.time)
    u16(entry.date)
    u32(entry.crc)
    u32(size)
    u32(size) // store: compressed size equals uncompressed size, no data descriptor
    u16(entry.nameBytes.length)
    u16(0) // no extra field
    copy(entry.nameBytes)
    copy(entry.data)
  }

  // Central directory: the same facts again, plus each entry's local offset.
  const centralOffset = cursor
  for (const entry of planned) {
    const size = entry.data.length
    u32(CENTRAL_SIGNATURE)
    // Version made by, host byte 0 (MS-DOS) then the 2.0 version byte.
    u16(ZIP_VERSION)
    u16(ZIP_VERSION)
    u16(FLAG_UTF8_NAME)
    u16(METHOD_STORE)
    u16(entry.time)
    u16(entry.date)
    u32(entry.crc)
    u32(size)
    u32(size)
    u16(entry.nameBytes.length)
    u16(0) // no extra field
    u16(0) // no comment
    u16(0) // starts on this disk
    u16(0) // internal attributes
    u32(0) // external attributes: DOS "normal file"
    u32(entry.offset)
    copy(entry.nameBytes)
  }

  // End of central directory: no zip64 record, no archive comment.
  u32(END_SIGNATURE)
  u16(0) // this disk
  u16(0) // disk holding the central directory
  u16(planned.length)
  u16(planned.length)
  u32(centralBytes)
  u32(centralOffset)
  u16(0)
  return archive
}
