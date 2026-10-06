/**
 * OIDC Relying-Party primitives for the identity seam (C3): discovery,
 * authorization-URL construction, code exchange, and ID-token verification.
 *
 * Zero dependencies by rule (新外部依赖需 Ask): HTTP via global fetch, JWT
 * signature verification via node:crypto (RS256 and ES256 — the two algs the
 * AliothMeta SSO and mainstream IdPs emit). JWKS keys are cached per URI with
 * a TTL; an unknown `kid` invalidates the cache once so a rotated key is
 * picked up on the next attempt.
 * @module @dsh-alioth/auth-alioth/oidc
 */

import { createPublicKey, verify as cryptoVerify } from 'node:crypto'

/** The endpoints the RP needs, from `/.well-known/openid-configuration`. */
export interface OidcDiscovery {
  readonly issuer: string
  readonly authorizationEndpoint: string
  readonly tokenEndpoint: string
  readonly jwksUri: string
}

/** One ID-token claim set (only what the mapping reads). */
export interface IdClaims {
  readonly sub: string
  readonly issuer: string
  readonly audience: string
  readonly preferredUsername: string | null
  readonly email: string | null
  readonly name: string | null
}

interface Jwk {
  readonly kid?: string
  readonly kty?: string
  readonly crv?: string
  readonly n?: string
  readonly e?: string
  readonly x?: string
  readonly y?: string
}

const DISCOVERY_TIMEOUT_MS = 5_000
const JWKS_TTL_MS = 10 * 60 * 1000

async function getJson(url: string): Promise<unknown> {
  const response = await fetch(url, { signal: AbortSignal.timeout(DISCOVERY_TIMEOUT_MS) })
  if (!response.ok) {
    throw new Error(`oidc: ${url} answered ${response.status}`)
  }
  return JSON.parse(await response.text()) as unknown
}

/**
 * Fetch and validate the provider's discovery document. `issuer` must match
 * the discovered `issuer` claim exactly — a mismatch means the URL points at
 * something that is not the IdP it claims to be.
 */
export async function discoverOidc(issuerUrl: string): Promise<OidcDiscovery> {
  const base = issuerUrl.replace(/\/+$/, '')
  const document = await getJson(`${base}/.well-known/openid-configuration`) as Record<string, unknown>
  const issuer = typeof document.issuer === 'string' ? document.issuer : ''
  const authorizationEndpoint = typeof document.authorization_endpoint === 'string' ? document.authorization_endpoint : ''
  const tokenEndpoint = typeof document.token_endpoint === 'string' ? document.token_endpoint : ''
  const jwksUri = typeof document.jwks_uri === 'string' ? document.jwks_uri : ''
  if (issuer !== base || authorizationEndpoint === '' || tokenEndpoint === '' || jwksUri === '') {
    throw new Error(
      `oidc: discovery document at ${base} is not a usable OIDC provider (issuer mismatch or missing endpoints)`
      + ` — check oidcIssuer (issuer=${JSON.stringify(issuer)})`,
    )
  }
  return { issuer, authorizationEndpoint, tokenEndpoint, jwksUri }
}

/** Build the authorization redirect (response_type=code; PKCE is not required for confidential clients). */
export function buildAuthorizeUrl(
  discovery: OidcDiscovery,
  options: { clientId: string; redirectUri: string; scope: string; state: string },
): string {
  const url = new URL(discovery.authorizationEndpoint)
  url.searchParams.set('response_type', 'code')
  url.searchParams.set('client_id', options.clientId)
  url.searchParams.set('redirect_uri', options.redirectUri)
  url.searchParams.set('scope', options.scope)
  url.searchParams.set('state', options.state)
  return url.toString()
}

/** Exchange the authorization code for tokens; only the ID token is kept. */
export async function exchangeCode(
  discovery: OidcDiscovery,
  options: { clientId: string; clientSecret: string; redirectUri: string; code: string },
): Promise<{ idToken: string }> {
  const body = new URLSearchParams({
    grant_type: 'authorization_code',
    code: options.code,
    redirect_uri: options.redirectUri,
    client_id: options.clientId,
    client_secret: options.clientSecret,
  })
  const response = await fetch(discovery.tokenEndpoint, {
    method: 'POST',
    headers: { 'content-type': 'application/x-www-form-urlencoded' },
    body,
    signal: AbortSignal.timeout(DISCOVERY_TIMEOUT_MS),
  })
  if (!response.ok) {
    throw new Error(`oidc: token endpoint answered ${response.status}`)
  }
  const tokens = JSON.parse(await response.text()) as Record<string, unknown>
  const idToken = typeof tokens.id_token === 'string' ? tokens.id_token : ''
  if (idToken === '') {
    throw new Error('oidc: token response carries no id_token')
  }
  return { idToken }
}

// ── JWKS cache ───────────────────────────────────────────────────────────

interface CacheEntry {
  readonly keys: readonly Jwk[]
  readonly loadedAt: number
}

const jwksCaches = new Map<string, CacheEntry>()

async function jwksFor(jwksUri: string, force = false): Promise<readonly Jwk[]> {
  const cached = jwksCaches.get(jwksUri)
  if (!force && cached !== undefined && Date.now() - cached.loadedAt < JWKS_TTL_MS) {
    return cached.keys
  }
  const document = await getJson(jwksUri) as { keys?: unknown }
  const keys = Array.isArray(document.keys) ? document.keys as Jwk[] : []
  if (keys.length === 0) {
    throw new Error(`oidc: JWKS at ${jwksUri} carries no keys`)
  }
  jwksCaches.set(jwksUri, { keys, loadedAt: Date.now() })
  return keys
}

/** Test seam: forget cached JWKS documents. */
export function clearJwksCache(): void {
  jwksCaches.clear()
}

function base64UrlDecode(segment: string): Buffer {
  return Buffer.from(segment.replace(/-/g, '+').replace(/_/g, '/'), 'base64')
}

/** Split and parse the token's signing input, header, and payload. */
function parseJwt(idToken: string): { signed: Buffer; header: Record<string, unknown>; payload: Record<string, unknown>; signature: Buffer } {
  const parts = idToken.split('.')
  if (parts.length !== 3) {
    throw new Error('oidc: id_token is not a JWS')
  }
  const signed = Buffer.from(`${parts[0]}.${parts[1]}`, 'utf8')
  const header = JSON.parse(base64UrlDecode(parts[0]!).toString('utf8')) as Record<string, unknown>
  const payload = JSON.parse(base64UrlDecode(parts[1]!).toString('utf8')) as Record<string, unknown>
  const signature = base64UrlDecode(parts[2]!)
  return { signed, header, payload, signature }
}

/**
 * Verify the ID token against the provider's JWKS and assert iss/aud/exp.
 * Throws with a reason on any failure — fail-closed, never "verify later".
 */
export async function verifyIdToken(
  idToken: string,
  expected: { discovery: OidcDiscovery; audience: string },
): Promise<IdClaims> {
  const { signed, header, payload, signature } = parseJwt(idToken)
  const alg = typeof header.alg === 'string' ? header.alg : ''
  if (alg !== 'RS256' && alg !== 'ES256') {
    throw new Error(`oidc: id_token alg ${JSON.stringify(alg)} is not accepted (RS256/ES256 only)`)
  }
  const kid = typeof header.kid === 'string' ? header.kid : undefined
  const findKey = (keys: readonly Jwk[]): Jwk | undefined =>
    keys.find(key => key.kty === 'EC' || key.kty === 'RSA') !== undefined && kid !== undefined
      ? keys.find(key => key.kid === kid)
      : keys.find(key => key.kty === 'EC' || key.kty === 'RSA')

  let keys = await jwksFor(expected.discovery.jwksUri)
  let jwk = findKey(keys)
  if (jwk === undefined && kid !== undefined) {
    // Unknown kid: the provider rotated its key — refresh once, then fail.
    keys = await jwksFor(expected.discovery.jwksUri, true)
    jwk = findKey(keys)
  }
  if (jwk === undefined || jwk.n === undefined && jwk.x === undefined) {
    throw new Error(`oidc: no JWKS key matches kid ${JSON.stringify(kid ?? '(none)')}`)
  }
  const keyObject = createPublicKey({ key: jwk as never, format: 'jwk' })
  // node:crypto verifies ECDSA signatures in DER form — exactly what a JWS
  // ES256 token carries, so both algorithms verify the segment as-is.
  const ok = alg === 'RS256'
    ? cryptoVerify('RSA-SHA256', signed, keyObject, signature)
    : cryptoVerify('sha256', signed, keyObject, signature)
  if (!ok) {
    throw new Error('oidc: id_token signature verification failed')
  }
  const now = Math.floor(Date.now() / 1000)
  const exp = typeof payload.exp === 'number' ? payload.exp : Number.NaN
  const iss = typeof payload.iss === 'string' ? payload.iss : ''
  const aud = typeof payload.aud === 'string' ? payload.aud : ''
  if (!Number.isFinite(exp) || exp < now) {
    throw new Error('oidc: id_token is expired')
  }
  if (iss !== expected.discovery.issuer) {
    throw new Error(`oidc: id_token issuer ${JSON.stringify(iss)} does not match the discovered issuer`)
  }
  if (aud !== expected.audience) {
    throw new Error(`oidc: id_token audience ${JSON.stringify(aud)} does not match the configured client`)
  }
  const sub = typeof payload.sub === 'string' ? payload.sub : ''
  if (sub === '') {
    throw new Error('oidc: id_token carries no sub claim')
  }
  const preferredUsername = typeof payload.preferred_username === 'string' ? payload.preferred_username : null
  const email = typeof payload.email === 'string' ? payload.email : null
  const name = typeof payload.name === 'string' ? payload.name : null
  return { sub, issuer: iss, audience: aud, preferredUsername, email, name }
}
