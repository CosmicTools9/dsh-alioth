/**
 * Source-download plumbing that is not pure: the entitlement rule, the deployment's
 * signing key, and the audit trail.
 *
 * The entitlement rule is the product's: a subscription covers the source download
 * **for the period it was paid for**. A canceled subscription keeps working until
 * `renewsAt` (the period is paid), and lapses on the first millisecond after it —
 * "订阅期内可下" and "链接短时有效" are two separate clocks, deliberately.
 * @module @dsh-alioth/auth-web-alioth/source-download
 */

import { appendFile, mkdir, open, readFile } from 'node:fs/promises'
import { randomBytes } from 'node:crypto'
import path from 'node:path'

/**
 * Structural face of an L2 source license — no dependency on the billing package,
 * and deliberately NOT a subscription: the gate below cannot see one.
 */
export interface SourceLicenseLike {
  /** Inclusive end of the negotiated authorization window. */
  readonly until: Date
}

/** Why access was granted or refused (reached through {@link SourceEntitlement}). */
type SourceEntitlementReason = 'licensed' | 'none' | 'expired'

/** The decision, with the date it holds until. */
export interface SourceEntitlement {
  readonly entitled: boolean
  /** ISO timestamp of the authorization's end (null when there is no license at all). */
  readonly until: string | null
  readonly reason: SourceEntitlementReason
}

/**
 * Decide whether an account may download source right now.
 *
 * An L1 subscription is NOT an input here on purpose: the published ladder sells
 * source as its own tier (L2, 商务对接), so a subscriber must not reach the package.
 * @param license - the account's L2 authorization, or null when it has none.
 * @param now - current time (injected for deterministic tests).
 * @returns the decision and the authorization end it depends on.
 */
export function sourceEntitlement(license: SourceLicenseLike | null, now: Date): SourceEntitlement {
  if (license === null) return { entitled: false, until: null, reason: 'none' }
  const until = license.until.toISOString()
  if (now.getTime() > license.until.getTime()) return { entitled: false, until, reason: 'expired' }
  return { entitled: true, until, reason: 'licensed' }
}

/** Signing key length in bytes. */
const KEY_BYTES = 32

/**
 * Load the deployment's source-link signing key, creating it on first use.
 *
 * Kept next to the other deployment state (0600) rather than in an env var: the
 * key must survive restarts and be identical across every process of one
 * deployment, and `ALIOTH_*` env plumbing already has enough surfaces. Creation is
 * exclusive (`wx`), so two processes racing at boot converge on one key instead of
 * clobbering each other.
 * @param file - absolute path of the key file.
 * @returns the key bytes.
 */
export async function loadSigningKey(file: string): Promise<Uint8Array> {
  const existing = await readFile(file).catch(() => null)
  if (existing !== null && existing.length >= KEY_BYTES) return existing

  await mkdir(path.dirname(file), { recursive: true })
  const key = randomBytes(KEY_BYTES)
  try {
    const handle = await open(file, 'wx', 0o600)
    try {
      await handle.writeFile(key)
    } finally {
      await handle.close()
    }
    return key
  } catch {
    // Lost a boot race: whoever wrote first owns the key.
    const written = await readFile(file)
    return written
  }
}

/** One audit line per redeemed download. */
export interface SourceDownloadAudit {
  readonly ts: string
  readonly userId: string
  readonly username: string
  readonly namespace: string
  readonly app: string
  readonly bytes: number
  readonly files: number
  /** When the link that produced this download stops working. */
  readonly linkExpiresAt: string
}

/**
 * Append one audit line. Best-effort by design: a failed audit write must not fail
 * a download the user already paid for, so callers get the error and decide.
 * @param file - absolute path of the JSONL audit file.
 * @param entry - the line to append.
 */
export async function appendSourceDownloadAudit(file: string, entry: SourceDownloadAudit): Promise<void> {
  await mkdir(path.dirname(file), { recursive: true })
  // `appendFile` opens with O_APPEND: one line per call, no read-modify-write,
  // and concurrent downloads cannot drop each other's line.
  await appendFile(file, `${JSON.stringify(entry)}\n`, 'utf8')
}
