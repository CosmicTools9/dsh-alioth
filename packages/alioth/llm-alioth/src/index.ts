/**
 * `@dsh-alioth/llm-alioth` — the BYOK-aware DeepSeek provider route.
 *
 * Replaces the harness `llm-deepseek` row (same provider id `deepseek-official`,
 * same settings namespace) with ONE behavioral difference: per-request key
 * resolution consults the signed-in account's own API key first
 * (`ctx.aliothAuth.apiKeyFor`, the BYOK seam), then falls back to the platform
 * credential exactly like the harness row. On the B/S deployment this is what
 * makes 用量→计费 honest end to end: an account that brought its own key burns
 * its own quota, the platform key only backs accounts without one.
 *
 * The per-request identity comes from the harness connection scope
 * (`currentConnectionAccount()` — present inside every `/api` dispatch and
 * remote.mux stream; absent in headless, where the platform key applies).
 * @module @dsh-alioth/llm-alioth
 */

import type { Context } from '@deepseek-ai/cordis'
import { assertUsableApiKey, LlmError } from '@deepseek-ai/dsh-llm'
import { launchEnvironmentOf } from '@deepseek-ai/dsh-launch-environment'
import { catalogModelInfo, registerDeepSeekProvider } from '@deepseek-ai/dsh-llm-deepseek'
import { Config, plainOptions, resolveAdapterOptions } from '@deepseek-ai/dsh-llm-deepseek-api-key'
import type { ResolvedDeepSeekOptions } from '@deepseek-ai/dsh-llm-deepseek-api-key'

export { Config, plainOptions, resolveAdapterOptions } from '@deepseek-ai/dsh-llm-deepseek-api-key'
export type { Options, ResolvedDeepSeekOptions } from '@deepseek-ai/dsh-llm-deepseek-api-key'

export const name = 'llm-alioth'
export const inject = ['llm']

const PROVIDER = 'deepseek-official'

/** BYOK key source: the structural face of `ctx.aliothAuth` this plugin reads. */
export interface AuthKeySource {
  apiKeyFor?(account: string): Promise<string | null>
}

/** Key-resolution inputs, gathered by `apply` and injected for tests. */
export interface ResolveKeyOptions {
  /** The dispatch's signed-in account (namespace or username), or null. */
  readonly account: string | null
  /** BYOK seam (optional; absent ⇒ platform credential only). */
  readonly apiKeyFor?: ((account: string) => Promise<string | null>) | undefined
  /** The harness credentials service, when mounted. */
  readonly credentials?: { resolve(ref: string): Promise<{ value: string } | undefined> } | undefined
  /** The ambient environment value of the credential ref (no credentials service). */
  readonly ambient: string | undefined
  /** Credential reference (usually DEEPSEEK_API_KEY). */
  readonly ref: string
}

/** Where the request key came from (usage attribution / diagnostics). */
export type KeySource = 'byok' | 'credentials' | 'ambient'

/**
 * The per-request key order: BYOK → credentials store → ambient environment.
 * Throws the same MISSING_CREDENTIAL LlmError the harness row throws when
 * nothing usable is configured — never a silent empty key.
 */
export async function resolveRequestKey(options: ResolveKeyOptions): Promise<{ key: string; source: KeySource }> {
  if (options.account !== null && options.apiKeyFor !== undefined) {
    const own = await options.apiKeyFor(options.account)
    if (own !== null && own !== '') {
      return { key: assertUsableApiKey(own, 'llm-alioth', `account:${options.account}`), source: 'byok' }
    }
  }
  if (options.credentials !== undefined) {
    const hit = await options.credentials.resolve(options.ref)
    if (hit !== undefined) {
      return { key: assertUsableApiKey(hit.value, 'llm-alioth', options.ref), source: 'credentials' }
    }
  } else if (options.ambient !== undefined && options.ambient.length > 0) {
    return { key: assertUsableApiKey(options.ambient, 'llm-alioth', options.ref), source: 'ambient' }
  }
  throw new LlmError(
    `llm-alioth: no API key for provider route "${PROVIDER}"; the account ${JSON.stringify(options.account ?? '(anonymous)')}`
    + ` has no BYOK key and the platform key is unconfigured — store ${options.ref} through the credentials`
    + ' service (the web Models page writes it), or export it in the launching environment',
    'MISSING_CREDENTIAL',
  )
}

/**
 * The signed-in account of the dispatch currently being processed. Loaded
 * dynamically: `@deepseek-ai/dsh-client-connection` is a web-profile package,
 * and a missing module means "no account in scope", never a failure.
 */
async function connectionAccount(): Promise<string | null> {
  try {
    const mod = await import('@deepseek-ai/dsh-client-connection')
    return mod.currentConnectionAccount()
  } catch {
    return null
  }
}

export function apply(ctx: Context, config: Config): void {
  const options = (): ResolvedDeepSeekOptions => resolveAdapterOptions(plainOptions(config), launchEnvironmentOf(ctx))
  options()
  const authKeys = (): AuthKeySource | undefined => {
    try {
      const value = (ctx.get as (name: string) => unknown).call(ctx, 'aliothAuth')
      return typeof value === 'object' && value !== null && typeof (value as AuthKeySource).apiKeyFor === 'function'
        ? value as AuthKeySource
        : undefined
    } catch {
      return undefined
    }
  }
  const credentialsService = (): { resolve(ref: string): Promise<{ value: string } | undefined> } | undefined => {
    try {
      const value = (ctx.get as (name: string) => unknown).call(ctx, 'credentials')
      return typeof value === 'object' && value !== null && typeof (value as { resolve?: unknown }).resolve === 'function'
        ? value as { resolve(ref: string): Promise<{ value: string } | undefined> }
        : undefined
    } catch {
      return undefined
    }
  }
  const resolveApiKey = async (connection: ResolvedDeepSeekOptions): Promise<string> => {
    const { key } = await resolveRequestKey({
      account: await connectionAccount(),
      apiKeyFor: authKeys()?.apiKeyFor,
      credentials: credentialsService(),
      ambient: launchEnvironmentOf(ctx).get(connection.apiKeyEnv)?.value,
      ref: connection.apiKeyEnv,
    })
    return key
  }
  // Same settings namespace as the harness row this replaces: the web Models
  // page (and any deployment settings under `llm-deepseek:`) keep working.
  ctx.llm.registerConfigurableProviders([
    { provider: PROVIDER, displayName: 'DeepSeek', settingsNs: 'llm-deepseek', settingsPath: [] },
  ])
  registerDeepSeekProvider(ctx, PROVIDER, {
    options,
    providerName: 'DeepSeek',
    resolveAuth: async connection => ({ headers: { 'x-api-key': await resolveApiKey(connection) } }),
    discoverModels: provider => {
      const connection = options()
      return Promise.resolve(connection.models.map(model => catalogModelInfo(provider, model)))
    },
  })
}
