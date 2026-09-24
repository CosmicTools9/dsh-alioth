/**
 * Time-limited source-download links.
 *
 * Two clocks govern a download, and this module carries both: the **entitlement**
 * window (the caller's subscription period, checked when the link is issued) and
 * the **link window** (a short TTL, checked when it is redeemed). A link is bound
 * to one account: the payload carries the user id, and redemption requires that
 * same session — a forwarded URL is useless to anyone else.
 *
 * Format: `base64url(payload) + '.' + base64url(hmac-sha256(payload))`. The payload
 * is serialized by this module alone (fixed key order), so signing and verifying
 * agree byte for byte without a canonical-JSON dependency.
 * @module @dsh-alioth/auth-web-alioth/source-link
 */

import { createHmac, timingSafeEqual } from 'node:crypto'

/** What a source-download link authorises. */
export interface SourceLinkPayload {
  /** Owning namespace (`U-<username>`). */
  readonly namespace: string
  /** App code inside that namespace. */
  readonly app: string
  /** Account the link is bound to — redemption requires this same session. */
  readonly userId: string
  /** Unix seconds after which the link is dead (`Math.floor(now/1000)`). */
  readonly expiresAt: number
}

/** Why a token was refused (reached through {@link SourceLinkCheck}). */
type SourceLinkFailure = 'malformed' | 'signature' | 'expired'

/** Verification outcome. */
export type SourceLinkCheck =
  | { readonly ok: true; readonly payload: SourceLinkPayload }
  | { readonly ok: false; readonly reason: SourceLinkFailure }

/** Fixed key order — the signing input, and the only serialization used. */
function serialize(payload: SourceLinkPayload): string {
  return JSON.stringify([payload.namespace, payload.app, payload.userId, payload.expiresAt])
}

function deserialize(text: string): SourceLinkPayload | null {
  let parsed: unknown
  try {
    parsed = JSON.parse(text)
  } catch {
    return null
  }
  if (!Array.isArray(parsed) || parsed.length !== 4) return null
  const [namespace, app, userId, expiresAt] = parsed as unknown[]
  if (typeof namespace !== 'string' || typeof app !== 'string' || typeof userId !== 'string') return null
  if (typeof expiresAt !== 'number' || !Number.isInteger(expiresAt)) return null
  return { namespace, app, userId, expiresAt }
}

function sign(secret: Uint8Array, text: string): Buffer {
  return createHmac('sha256', secret).update(text).digest()
}

/**
 * Issue a link token.
 * @param secret - deployment signing key.
 * @param payload - the authorization being granted.
 * @returns the token to hand to the client.
 */
export function signSourceLink(secret: Uint8Array, payload: SourceLinkPayload): string {
  const body = Buffer.from(serialize(payload), 'utf8').toString('base64url')
  return `${body}.${sign(secret, body).toString('base64url')}`
}

/**
 * Verify a link token: shape first, then signature, then expiry — a forgery must
 * never be reported as "expired", and an expired token never as a signature miss.
 * @param secret - deployment signing key.
 * @param token - the token under test.
 * @param nowSeconds - current unix seconds (injected for deterministic tests).
 * @returns the payload, or the reason it was refused.
 */
export function verifySourceLink(secret: Uint8Array, token: string, nowSeconds: number): SourceLinkCheck {
  const dot = token.indexOf('.')
  if (dot <= 0 || dot === token.length - 1) return { ok: false, reason: 'malformed' }
  const body = token.slice(0, dot)
  const provided = Buffer.from(token.slice(dot + 1), 'base64url')
  const expected = sign(secret, body)
  if (provided.length !== expected.length || !timingSafeEqual(provided, expected)) {
    return { ok: false, reason: 'signature' }
  }
  const payload = deserialize(Buffer.from(body, 'base64url').toString('utf8'))
  if (payload === null) return { ok: false, reason: 'malformed' }
  if (payload.expiresAt <= nowSeconds) return { ok: false, reason: 'expired' }
  return { ok: true, payload }
}
