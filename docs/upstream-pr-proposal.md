# Upstream PR proposal: workspace UX policies + session identity for the web GUI

**Repo**: deepseek-ai/deepseek-harness
**Motivation**: dsh-alioth (AppCreator: an Alioth-model app generator built on the
harness web profile) currently carries a small client patch set
(`dsh-alioth/scripts/harness-patches/ui-workspace-pick-gate.patch`) to express
product-level workspace UX. All three concerns are generic multi-workspace /
multi-tenant needs; making them configuration-driven removes the fork.

---

## PR 1 — Configurable New-Session policy (choose a workspace first)

**File**: `packages/client/ui-workspace/src/client/navigation.ts`
(+ plugin Config plumbing in `packages/client/ui-workspace/src/client/index.ts`)

**Today**: `startSession()` without an explicit workspace id silently reconnects
the *recent* workspace (`workspaceId ?? currentWorkspaceId ?? recent`), so the
picker (hero empty state) is only reachable when no workspace exists at all.

**Problem**: products where "new chat" must mean "choose the target workspace
first" (every chat belongs to a workspace; the workspace *is* the product
object) cannot express that policy.

**Proposal**: add a client-plugin config flag, e.g.
`UiWorkspaceConfig.forceWorkspacePickOnNewSession?: boolean`. When set, a bare
`startSession()` (no explicit workspace id) clears the current session and
lands on the picker instead of reconnecting the recent workspace. Explicit
ids (picker result, workspace context menus) keep their current path.

**Compatibility**: default `false` — behaviour unchanged. Client plugins
already receive loader-row config; this just adds the first Config to
ui-workspace.

## PR 2 — Scope workspace/session lists to an owning identity

**Files**:
- `packages/client/ui-workspace/src/client/rows/WorkspaceBrowser.tsx`
- `packages/client/ui-workspace/src/client/WorkspacePicker.tsx`
- `packages/client/ui-workspace/src/client/navigation.ts` (helper)

**Today**: workspace lists render every workspace in the registry; ungrouped
sessions render regardless of ownership. On a shared host (several products /
accounts using one `$DSH_HOME`) this leaks other owners' workspaces and
sessions into the UI.

**Problem**: no ownership model on the client list path. Workspace records
carry a `path`; hosts that lay out workspaces under an owner segment
(`Pre-Proc/U-<owner>/…`) can scope by path, but the client has no hook.

**Proposal**: a configurable scope predicate on the workspace list store:
`UiWorkspaceConfig.workspaceVisible?: (workspace) => boolean` (or a simpler
`namespaceFilter?: string` that matches a path segment, kept generic: any
string the host sets; matching = path segment equality). The browser,
the picker and the ungrouped-session derivation apply it; absent config =
current behaviour. Host deployments that need per-account scoping set the
filter per composition (the web gate already knows the signed-in account).

**Rationale**: full per-user access control belongs in the host registry /
auth layer (see PR 3) — this is the read-side UI scope so hosts with their
own identity carrier can isolate immediately.

## PR 3 — Session identity: let the browser connection own a user

**Files** (investigation needed before writing):
- `packages/api/session-controller` (create/owner)
- `packages/client/connection` / `packages/api/gateway` (per-connection
  identity carrier)

**Today**: agent sessions carry no owner (`SessionCreateRequest` has no user
field; the `user` seen in some deployments is set by a host extension, not the
core). dsh-alioth worked around this by (a) a client-side POST to its own
`/api/auth/bind` after `session/create`, later (b) deriving the owner from the
session's workspace path (`Pre-Proc/U-<owner>/…`). Both are consumer-side
hacks for a core gap: **the web GUI has no "signed-in user" concept** —
browser auth is a per-device token, not an account.

**Proposal**: add an optional account identity to the browser connection:
- the connection layer already authenticates each request (device cookie +
  Host/Origin fence); allow hosts to also supply an *account* claim (e.g. a
  signed marker cookie set by the host's own login page — no core UI change),
- expose it on the session-creation path so a created session records
  `owner` (string, opaque),
- `session.create` request may then carry/record the owner from the
  connection identity instead of trusting client input.

Once sessions carry an owner, PR 2's read-side filter can key off it
natively, and hosts get authorization hooks (`tools/pre-execute` style)
without client patches.

**Scope note**: the account carrier (login pages, account cookies) stays a
host/plugin concern — this PR only wires the *channel* (connection identity →
session owner) and the *read model* (owner on session/workspace list rows).

---

## How this removes the dsh-alioth patch set

| dsh-alioth patch concern | Replaced by |
|---|---|
| force pick on new session | PR 1 flag (`forceWorkspacePickOnNewSession: true`) |
| namespace filter in browser/picker/ungrouped | PR 2 config (`namespaceFilter`) |
| picker hides folder controls | already non-invasive (backend capability) or PR 1/2 client config `allowFolderActions: false` |
| client-side session binding | PR 3 connection account → session owner; plus dsh-alioth's own path-derivation fallback stays as a consumer-side convenience |

**Alternatives considered**: overriding via localStorage flags (current dsh-alioth
approach — works, but untestable in client specs and invisible to the plugin
config surface); patching the loader row config per deployment (no client
Config exists today for ui-workspace — PR 1 introduces the pattern).
