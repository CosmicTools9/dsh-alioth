/**
 * Prototype content-root provisioning. The vendored prototype chain
 * (`scripts/prototype-tool.js` + `scripts/check/*`) expects the upstream
 * repo-root layout: `Pre-Proc/` (artifact tree), `.agents/skills/alioth-design/
 * references/` (design tokens, shells, icon pool) and `Framework/frontend/
 * components/utilities.json` (utility registry) all under one root, with the
 * tool invoked relative to that root. Deployments point PROTOTYPE_TOOL_ROOT at
 * this content root (default: the parent of the Pre-Proc root) and the
 * provisioner materializes the vendored pieces there — idempotent, never
 * overwriting existing files.
 * @module @dsh-alioth/env-alioth/prototype-root
 */

import { cpSync, existsSync, lstatSync, mkdirSync, rmSync, statSync, symlinkSync, unlinkSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

/** Vendored assets shipped inside the env-alioth package. */
export const VENDOR_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', 'vendor')

export interface PrototypeRootInfo {
  /** Content root (PROTOTYPE_TOOL_ROOT): parent of the Pre-Proc root. */
  readonly contentRoot: string
  /** The Pre-Proc artifact root as given. */
  readonly preProcRoot: string
  /** True when `contentRoot/Pre-Proc` is a symlink created by the provisioner. */
  readonly preProcLinked: boolean
}

function isDir(p: string): boolean {
  try {
    return statSync(p).isDirectory()
  } catch {
    return false
  }
}

/**
 * Materialize the vendored repo-root layout under `contentRoot`:
 * - `.agents/`, `Framework/`, `scripts/` copied from vendor when missing
 *   (never overwritten — local edits win);
 * - `Pre-Proc` symlinked to `preProcRoot` when the two differ (a pre-existing
 *   real directory is kept untouched — native layout).
 * Idempotent: safe to call on every gate run.
 */
export function provisionPrototypeRoot(preProcRoot: string, contentRoot = path.dirname(path.resolve(preProcRoot))): PrototypeRootInfo {
  const resolvedContent = path.resolve(contentRoot)
  const resolvedPreProc = path.resolve(preProcRoot)
  mkdirSync(resolvedContent, { recursive: true })

  // Merge copy: fills in files the content root is missing (e.g. a vendor
  // sync added Framework/backend) while keeping every pre-existing file.
  for (const dir of ['.agents', 'Framework', 'scripts']) {
    const target = path.join(resolvedContent, dir)
    const source = path.join(VENDOR_ROOT, dir)
    if (existsSync(source)) {
      mkdirSync(path.dirname(target), { recursive: true })
      cpSync(source, target, { recursive: true, force: false })
    }
  }

  // The vendored Framework crates inherit [workspace.package] from the
  // upstream repo-root manifest (AliothStudio/Cargo.toml). Provision a
  // minimal root workspace so that inheritance resolves without the
  // AliothStudio checkout — Pre-Proc trees stay excluded from it.
  const rootManifest = path.join(resolvedContent, 'Cargo.toml')
  if (!existsSync(rootManifest)) {
    writeFileSync(
      rootManifest,
      '# Provisioned content-root workspace (dsh-alioth): resolves the vendored\n'
      + '# Framework crates without the AliothStudio checkout.\n'
      + '[workspace]\n'
      + 'resolver = "2"\n'
      + 'members = ["Framework/backend/*"]\n'
      + 'exclude = ["Pre-Proc/**"]\n'
      + '\n'
      + '[workspace.dependencies]\n'
      + 'tokio = { version = "1", features = ["full"] }\n'
      + 'actix-web = "4"\n'
      + 'sqlx = { version = "0.9.0", features = ["runtime-tokio", "postgres", "uuid", "chrono", "macros", "migrate", "rust_decimal"] }\n'
      + 'serde = { version = "1", features = ["derive"] }\n'
      + 'serde_json = "1"\n'
      + 'chrono = { version = "0.4", features = ["serde"] }\n'
      + 'uuid = { version = "1", features = ["v4", "serde"] }\n'
      + 'yaml_serde = "0.10"\n'
      + 'thiserror = "2"\n'
      + 'anyhow = "1"\n'
      + 'base64 = "0.23"\n'
      + 'aes-gcm = "0.11"\n'
      + 'rand = "0.10"\n'
      + 'petgraph = { version = "0.8", features = ["serde"] }\n'
      + 'bcrypt = "0.19"\n'
      + 'jsonwebtoken = { version = "10", features = ["rust_crypto"] }\n'
      + 'log = "0.4"\n'
      + 'actix-cors = "0.7"\n'
      + 'dotenvy = "0.15"\n'
      + 'futures = "0.3"\n'
      + 'futures-util = "0.3"\n'
      + 'zip = "2"\n'
      + 'convert_case = "0.11"\n'
      + 'pluralizer = "0.5"\n'
      + 'rust_decimal = { version = "1", features = ["serde"] }\n'
      + 'rust_decimal_macros = "1"\n'
      + 'moka = { version = "0.12", features = ["future"] }\n'
      + 'reqwest = { version = "0.13", features = ["json"] }\n'
      + 'regex = "1"\n'
      + 'walkdir = "2"\n'
      + 'crc32fast = "1"\n'
      + 'pdf-extract = "0.12"\n'
      + 'async-trait = "0.1"\n'
      + 'sha2 = "0.11"\n'
      + 'p256 = { version = "0.14", features = ["ecdh"] }\n'
      + 'criterion = { version = "0.8", features = ["html_reports"] }\n'
      + '\n'
      + 'urlencoding = "2"\n'
      + 'url = "2"\n'
      + 'totp-rs = { version = "5.7", features = ["otpauth"] }\n'
      + 'toml = "0.8"\n'
      + 'time = "0.3"\n'
      + 'tempfile = "3"\n'
      + 'similar = "3.1"\n'
      + 'prometheus = "0.14"\n'
      + 'md5 = "0.8"\n'
      + 'ldap3 = "0.12"\n'
      + 'json5 = "1.3"\n'
      + 'hex = "0.4"\n'
      + 'env_logger = "0.11"\n'
      + 'dashmap = "6"\n'
      + 'arc-swap = "1"\n'
      + 'base32 = "0.5"\n'
      + 'argon2 = "0.6.0-rc.8"\n'
      + 'actix-web-actors = "4"\n'
      + 'actix-rt = "2"\n'
      + 'actix = "0.13"\n'
      + '\n'
      + '# Deps migrated from Framework/Cargo.toml\n'
      + 'bincode = "1.3"\n'
      + 'bytes = "1.11"\n'
      + 'crc = "3"\n'
      + 'crossbeam-utils = "0.8"\n'
      + 'disruptor = "4.3.0"\n'
      + 'ecdsa = "0.17"\n'
      + 'fs2 = "0.4"\n'
      + 'mio = { version = "1.1", features = ["net", "os-poll", "os-ext"] }\n'
      + 'once_cell = "1.21"\n'
      + 'pem = "3"\n'
      + 'proc-macro2 = "1.0"\n'
      + 'quote = "1.0"\n'
      + 'ring = "0.17"\n'
      + 'rustls = { version = "0.23", default-features = false, features = ["ring", "std"] }\n'
      + 'rustls-native-certs = "0.8"\n'
      + 'rustls-pemfile = "2"\n'
      + 'sha1 = "0.11"\n'
      + 'sha3 = "0.10"\n'
      + 'subtle = "2.6"\n'
      + 'tokio-postgres = "0.7"\n'
      + 'aws-sdk-s3 = "1.67"\n'
      + 'aws-config = { version = "1.5", features = ["behavior-version-latest"] }\n'
      + '\n'
      + '# ---------------------------------------------------------------------------\n'
      + '# PATCH: webauthn-rs-core 0.5.5 passes the assertion signature straight to\n'
      + '# OpenSSL EVP verify, which expects DER (X9.62) ECDSA signatures, while the\n'
      + '# WebAuthn standard mandates raw 64-byte r||s. Real browsers therefore fail\n'
      + '# passkey login with "An OpenSSL Error". vendor/webauthn-rs-core adds a\n'
      + '# raw->DER conversion for ES256 in pkey_verify_signature.\n'
      + '# ---------------------------------------------------------------------------\n'
      + '[workspace.package]\n'
      + 'version = "0.1.0"\n'
      + 'edition = "2021"\n'
      + 'authors = ["The Alioth Authors"]\n'
      + 'license = "Apache-2.0"\n',
    )
  }

  let preProcLinked = false
  const preProcLink = path.join(resolvedContent, 'Pre-Proc')
  if (resolvedPreProc !== preProcLink) {
    let state: 'dir' | 'symlink' | 'missing' = 'missing'
    try {
      state = lstatSync(preProcLink).isSymbolicLink() ? 'symlink' : isDir(preProcLink) ? 'dir' : 'missing'
    } catch {
      state = 'missing'
    }
    if (state === 'symlink') {
      preProcLinked = true
    } else if (state === 'missing') {
      mkdirSync(resolvedPreProc, { recursive: true })
      symlinkSync(resolvedPreProc, preProcLink, 'dir')
      preProcLinked = true
    }
  }

  return { contentRoot: resolvedContent, preProcRoot: resolvedPreProc, preProcLinked }
}

/** Remove a provisioned content root's copied assets (tests). */
export function removeProvisionedAssets(contentRoot: string): void {
  for (const dir of ['.agents', 'Framework', 'scripts']) {
    rmSync(path.join(contentRoot, dir), { recursive: true, force: true })
  }
  const preProcLink = path.join(contentRoot, 'Pre-Proc')
  try {
    if (lstatSync(preProcLink).isSymbolicLink()) {
      unlinkSync(preProcLink)
    }
  } catch {
    // not present / not a link
  }
}
