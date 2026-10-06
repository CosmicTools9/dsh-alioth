/**
 * BYOK secret box: user-provided model API keys must be recoverable (they are
 * sent to the LLM provider), so unlike passwords they are stored ENCRYPTED,
 * not hashed — AES-256-GCM under a deployment secret. Format:
 * `v1:<ivB64>:<tagB64>:<cipherB64>`. The key is SHA-256 of the deployment
 * secret; the deployment secret comes from `ALIOTH_BYOK_SECRET` (env) or the
 * plugin config. Without one, BYOK is DISABLED (fail-closed): storing fails
 * loud, reading answers null.
 * @module @dsh-alioth/auth-alioth/secretbox
 */

import { createCipheriv, createDecipheriv, createHash, randomBytes } from 'node:crypto'

const VERSION = 'v1'

function keyOf(secret: string): Buffer {
  return createHash('sha256').update(secret, 'utf8').digest()
}

/** Encrypt an API key, or null when no deployment secret is configured. */
export function sealSecret(plain: string, secret: string | undefined): string | null {
  if (secret === undefined || secret === '') return null
  const iv = randomBytes(12)
  const cipher = createCipheriv('aes-256-gcm', keyOf(secret), iv)
  const encrypted = Buffer.concat([cipher.update(plain, 'utf8'), cipher.final()])
  const tag = cipher.getAuthTag()
  return `${VERSION}:${iv.toString('base64')}:${tag.toString('base64')}:${encrypted.toString('base64')}`
}

/** Decrypt a stored key; null when absent, malformed, or tampered with. */
export function openSecret(sealed: string | null, secret: string | undefined): string | null {
  if (sealed === null || secret === undefined || secret === '') return null
  const parts = sealed.split(':')
  if (parts.length !== 4 || parts[0] !== VERSION) return null
  try {
    const decipher = createDecipheriv('aes-256-gcm', keyOf(secret), Buffer.from(parts[1]!, 'base64'))
    decipher.setAuthTag(Buffer.from(parts[2]!, 'base64'))
    return Buffer.concat([decipher.update(Buffer.from(parts[3]!, 'base64')), decipher.final()]).toString('utf8')
  } catch {
    return null
  }
}
