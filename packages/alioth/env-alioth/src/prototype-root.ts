/**
 * Prototype content-root provisioning. The vendored prototype chain
 * (`scripts/prototype-tool.js` + `scripts/check/*`) and the vendored Gateway
 * build expect the upstream repo-root layout: `Pre-Proc/` (artifact tree),
 * `.agents/skills/alioth-design/references/`, `Framework/`,
 * `Gateway/backend`, `SSO/backend` and `scripts/` — all under one root.
 * Deployments point PROTOTYPE_TOOL_ROOT at this content root (default: the
 * parent of the Pre-Proc root) and the provisioner materializes the vendored
 * pieces there — idempotent merge (never overwrites existing files), with
 * stub synthesis for gateway deps the deployment doesn't carry.
 * @module @dsh-alioth/env-alioth/prototype-root
 */

import { cpSync, existsSync, lstatSync, mkdirSync, readFileSync, rmSync, symlinkSync, unlinkSync, writeFileSync } from 'node:fs'
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

/** The vendored gateway manifest declares OPTIONAL path deps across ALL
 * upstream namespaces — capture dep name, target path and the optional
 * `package = "…"` rename. */
function gatewayPathDeps(manifestText: string): { readonly key: string; readonly pkgName: string; readonly relPath: string }[] {
  const deps: { key: string; pkgName: string; relPath: string }[] = []
  for (const match of manifestText.matchAll(/^([a-z0-9_-]+)\s*=\s*\{\s*path = "([^"]+)"([^}]*)\}/gm)) {
    deps.push({
      key: match[1] ?? '',
      pkgName: /package = "([^"]+)"/.exec(match[3] ?? '')?.[1] ?? match[1] ?? '',
      relPath: match[2] ?? '',
    })
  }
  return deps
}

const STUB_MARKER = 'Deployment stub (dsh-alioth provisioner)'

/**
 * Materialize the vendored repo-root layout under `contentRoot`:
 *
 * 1. link `contentRoot/Pre-Proc` → preProcRoot when the two differ;
 * 2. merge-copy the vendored trees (`Gateway`, `SSO`, `Pre-Proc/Alioth`,
 *    `.agents`, `Framework`, `scripts`) — fills in files the content root is
 *    missing (e.g. after a vendor sync) while keeping pre-existing files;
 * 3. synthesize stub crates for gateway optional path deps whose targets the
 *    deployment doesn't carry (other namespaces' services) — disabled
 *    features never compile their stubs, and a stub whose real vendored tree
 *    appears later is replaced on the next provision.
 *
 * Idempotent: safe to call on every gate run.
 */
export function provisionPrototypeRoot(preProcRoot: string, contentRoot = path.dirname(path.resolve(preProcRoot))): PrototypeRootInfo {
  const resolvedContent = path.resolve(contentRoot)
  const resolvedPreProc = path.resolve(preProcRoot)
  mkdirSync(resolvedContent, { recursive: true })

  // 1) Pre-Proc link first: vendored namespaces below must land inside the
  //    linked artifact root.
  let preProcLinked = false
  const preProcLink = path.join(resolvedContent, 'Pre-Proc')
  if (resolvedPreProc !== preProcLink) {
    // Recreate when missing (incl. a dangling symlink whose target was
    // removed); keep an existing valid link or a real directory.
    const validLink = (() => {
      try {
        return lstatSync(preProcLink).isSymbolicLink() && existsSync(preProcLink)
      } catch {
        return false
      }
    })()
    if (!validLink) {
      const isLink = (() => {
        try {
          return lstatSync(preProcLink).isSymbolicLink()
        } catch {
          return false
        }
      })()
      if (isLink) rmSync(preProcLink, { force: true })
      mkdirSync(resolvedPreProc, { recursive: true })
      symlinkSync(resolvedPreProc, preProcLink, 'dir')
      preProcLinked = true
    }
  }

  // 2) Merge copy — fills in what's missing, keeps pre-existing files.
  //    Pre-Proc merges through the REAL artifact root (never via the symlink —
  //    cpSync refuses to overwrite a symlinked directory even with force:false).
  for (const dir of ['Gateway', 'SSO', 'Pre-Proc', '.agents', 'Framework', 'scripts']) {
    const target = dir === 'Pre-Proc' ? resolvedPreProc : path.join(resolvedContent, dir)
    const source = path.join(VENDOR_ROOT, dir)
    if (existsSync(source)) {
      mkdirSync(path.dirname(target), { recursive: true })
      cpSync(source, target, { recursive: true, force: false })
    }
  }

  // 3) Gateway optional path deps: stub synthesis for still-missing targets.
  //    A stub whose real vendored tree appeared is removed in step 2's wake:
  //    if the stub manifest carries the marker and the vendor has the real
  //    tree, drop the stub so the merge copy below restores real code.
  const gatewayManifest = path.join(resolvedContent, 'Gateway', 'backend', 'Cargo.toml')
  if (existsSync(gatewayManifest)) {
    const deps = gatewayPathDeps(readFileSync(gatewayManifest, 'utf8'))
    for (const dep of deps) {
      // Canonical content-root location: drop upstream relative depth, place
      // under Pre-Proc/ (or wherever the tail starts) inside the content root.
      const depManifest = path.join(resolvedContent, dep.relPath.replace(/^(\.\.\/)+/, ''))
      // The dep path is relative to Gateway/backend INSIDE the content root;
      // the vendor copy mirrors the content-root layout from ITS root.
      const vendorReal = path.join(VENDOR_ROOT, 'Gateway', 'backend', dep.relPath)
      try {
        if (
          readFileSync(depManifest, 'utf8').includes(STUB_MARKER)
          && existsSync(vendorReal)
        ) {
          rmSync(path.dirname(depManifest), { recursive: true, force: true })
        }
      } catch {
        // not a stub / not present
      }
    }
  }

  // 3b) Merge copy again after stub removal (real code fills the gap).
  for (const dir of ['Gateway', 'SSO', 'Pre-Proc']) {
    const target = dir === 'Pre-Proc' ? resolvedPreProc : path.join(resolvedContent, dir)
    const source = path.join(VENDOR_ROOT, dir)
    if (existsSync(source)) {
      cpSync(source, target, { recursive: true, force: false })
    }
  }

  // 3c) Synthesize stubs for deps still missing.
  if (existsSync(gatewayManifest)) {
    const deps = gatewayPathDeps(readFileSync(gatewayManifest, 'utf8'))
    for (const dep of deps) {
      // relPath is the CRATE DIR (upstream manifests point at the crate, not
      // the manifest file): the stub crate lands at <crateDir>/{Cargo.toml, src/lib.rs}.
      const crateDir = path.join(resolvedContent, dep.relPath.replace(/^(\.\.\/)+/, ''))
      if (existsSync(path.join(crateDir, 'Cargo.toml'))) continue
      mkdirSync(path.join(crateDir, 'src'), { recursive: true })
      writeFileSync(
        path.join(crateDir, 'Cargo.toml'),
        `[package]\nname = "${dep.pkgName}"\nversion = "0.1.0"\nedition = "2021"\nlicense = "Apache-2.0"\n\n[lib]\npath = "src/lib.rs"\n\n# ${STUB_MARKER}\n`,
      )
      writeFileSync(path.join(crateDir, 'src', 'lib.rs'), `//! ${STUB_MARKER}.\n`)
    }
  }

  // 4) Minimal content-root workspace manifest: the vendored Framework and
  //    baseline-service crates inherit [workspace.package] from the upstream
  //    repo-root manifest — provision one so inheritance resolves without
  //    the AliothStudio checkout. User namespace workspaces stay excluded.
  const rootManifest = path.join(resolvedContent, 'Cargo.toml')
  if (!existsSync(rootManifest)) {
    writeFileSync(
      rootManifest,
      '# Provisioned content-root workspace (dsh-alioth): resolves the vendored\n'
      + '# Framework crates without the AliothStudio checkout.\n'
      + '[workspace]\n'
      + 'resolver = "2"\n'
      + 'members = ["Framework/backend/*", "Gateway/backend", "Pre-Proc/Alioth/Sources/Apps/Services/*/backend"]\n'
      + 'exclude = ["Pre-Proc/Demo/**", "Framework/backend/.cargo"]\n'
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

  return { contentRoot: resolvedContent, preProcRoot: resolvedPreProc, preProcLinked }
}

/** Remove a provisioned content root's copied assets (tests). */
export function removeProvisionedAssets(contentRoot: string): void {
  for (const dir of ['.agents', 'Framework', 'Gateway', 'SSO', 'scripts']) {
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
