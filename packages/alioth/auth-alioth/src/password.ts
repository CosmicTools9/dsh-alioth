/**
 * Password hashing for the auth plugin: Node's built-in scrypt, no
 * dependencies. Encoding: `scrypt$N$r$p$saltB64$hashB64` (self-describing,
 * parameter bumps stay backward compatible via verify's parse).
 * @module @dsh-alioth/auth-alioth/password
 */

import { randomBytes, scrypt as scryptCallback, timingSafeEqual } from 'node:crypto'

const N = 16384
const R = 8
const P = 1
const KEY_LENGTH = 64
const SALT_LENGTH = 16

function scryptAsync(password: string, salt: Buffer, keylen: number, options: { N: number; r: number; p: number }): Promise<Buffer> {
  return new Promise((resolve, reject) => {
    scryptCallback(password, salt, keylen, options, (error, key) => {
      if (error) {
        reject(error)
      } else {
        resolve(key)
      }
    })
  })
}

export function hashPassword(password: string): Promise<string> {
  const salt = randomBytes(SALT_LENGTH)
  return scryptAsync(password, salt, KEY_LENGTH, { N, r: R, p: P }).then(key =>
    `scrypt$${N}$${R}$${P}$${salt.toString('base64')}$${key.toString('base64')}`)
}

export async function verifyPassword(password: string, encoded: string): Promise<boolean> {
  const parts = encoded.split('$')
  if (parts.length !== 6 || parts[0] !== 'scrypt') {
    return false
  }
  const [, n, r, p, saltB64, hashB64] = parts
  const expected = Buffer.from(hashB64!, 'base64')
  const actual = await scryptAsync(password, Buffer.from(saltB64!, 'base64'), expected.length, {
    N: Number(n), r: Number(r), p: Number(p),
  })
  return timingSafeEqual(actual, expected)
}
