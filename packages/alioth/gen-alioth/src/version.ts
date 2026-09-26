/**
 * Model-version dependency: every artifact that can declare one states the minimum
 * model it needs — `app.json.min_alioth_version`, `block.json` / `service.json`
 * `aliothVersion` — while the deployment provides a concrete model. This module is the
 * single place that parses and compares the two: generation stamps the live version,
 * display surfaces (`alioth_app_inspect`, the console's app-status panel) report the
 * dependency, and neither re-implements the comparison.
 *
 * `module.json` is deliberately absent: the upstream MODULE_SPEC declares no model-version
 * key, so a module's dependency is inherited from its owning app rather than invented here.
 * @module @dsh-alioth/gen-alioth/version
 */

/**
 * Floor declared by artifacts when the deployment's own model version is unknown
 * (the env service is not mounted). Upstream's model line: any v10 artifact runs on v10.0.0.
 */
export const DEFAULT_MIN_ALIOTH_VERSION = '10.0.0'

const RELEASE_RE = /^v?(\d+)\.(\d+)\.(\d+)$/

/** Parse a release-shaped `X.Y.Z` (a leading `v` is tolerated); null for fixtures, git refs, empty. */
export function parseModelVersion(value: string): readonly [number, number, number] | null {
  const match = RELEASE_RE.exec(value.trim())
  if (match === null) return null
  return [Number(match[1]), Number(match[2]), Number(match[3])]
}

/** Three-way compare of release-shaped versions; null when either side is not one. */
export function compareModelVersions(left: string, right: string): -1 | 0 | 1 | null {
  const a = parseModelVersion(left)
  const b = parseModelVersion(right)
  if (a === null || b === null) return null
  for (const index of [0, 1, 2] as const) {
    if (a[index] !== b[index]) return a[index] < b[index] ? -1 : 1
  }
  return 0
}

/** Does the deployment's `available` model satisfy a declared minimum? null = undecidable. */
export function satisfiesModelVersion(declared: string, available: string): boolean | null {
  const order = compareModelVersions(declared, available)
  return order === null ? null : order <= 0
}

/**
 * The version to stamp into a generated artifact: the deployment's own model when it is
 * release-shaped, else the floor. Normalised to bare digits — the live version arrives from
 * a model source that may spell it `v10.0.34` (the publication version) and the artifacts
 * carry `10.0.34`.
 * @param live - the deployment's model version, when known.
 */
export function modelVersionAnchor(live: string | undefined): string {
  const parsed = live === undefined ? null : parseModelVersion(live)
  return parsed === null ? DEFAULT_MIN_ALIOTH_VERSION : parsed.join('.')
}

/**
 * A version as a display surface must show it: bare digits when it is release-shaped (`v10.0.34`
 * and `10.0.34` are the same model), verbatim otherwise — a fixture or git-ref source is named as
 * it is rather than dressed up as a release. Artifacts and readbacks therefore agree.
 */
export function displayModelVersion(value: string): string {
  return parseModelVersion(value)?.join('.') ?? value
}
