# Prototype-to-Reference Evaluation Rubric

> **Purpose**: score any generated Alioth prototype (`m-v{N}.html`, `b-v{N}.html`, `a-v{N}.html`) against `gateway-shell.tsx` (v2.0.0) visual and functional contract, while allowing a completely different implementation toolchain (Tailwind + esbuild instead of Babel + inline `gl-gateway-*`).

This rubric is paired with the static evaluator:

```bash
bun scripts/eval/evaluate-prototype-reference.ts <prototype.html>
bun scripts/eval/evaluate-prototype-reference.ts <prototype.html> --human
```

Visual and functional parity (navigation switching, sidebar collapse, theme toggle, etc.) MUST be verified with the existing ego-browser pipeline:

```bash
bun scripts/check/check-visual-verify.ts <prototype.html>
```

## 1. Rubric Dimensions

| Dimension                    | Weight | Static Evaluator                                                                      | PASS threshold |
| ---------------------------- | ------ | ------------------------------------------------------------------------------------- | -------------- |
| **Structural Shell**         | 0.25   | ✅ `#boot-skeleton` layout, `#root` mount target, vendor scripts, layout variables    | ≥ 4/5          |
| **Resource References**      | 0.15   | ✅ Local fonts, prototype-base.css (link or inline; tailwind-utilities.css legacy OK), no external CDN              | ≥ 4/5          |
| **CSS Tokens**               | 0.20   | ✅ `:root` variables, required tokens, stray hardcoded color warnings                 | ≥ 4/5          |
| **Prohibited Patterns**      | 0.15   | ✅ No `gl-gateway-*` classes, no `react-router`, no `scrollIntoView`              | = 5/5          |
| **Build Metadata**           | 0.05   | ✅ Version comment, resolved scene list (no `scenes: ?`)                              | = 5/5          |
| **Existing Gate Compliance** | 0.20   | ✅ `ontology-mapping prototype-check`, `audit-css-framework.mjs`, `check-class-names.js` | = 5/5          |

**Overall PASS**: weighted score ≥ 90 and every dimension ≥ its threshold.

## 2. Static Checks by Dimension

### 2.1 Structural Shell

The generated HTML must contain:

- `#boot-skeleton` with sidebar, main, and loader placeholders
- `#root` mount target for React
- At least 3 vendor React scripts (react, react-dom, react-dom-client)
- CSS layout variables: `--topbar-height`, `--sidebar-width`, `--sidebar-collapsed-width`

These checks confirm the shell is ready to render the same Gateway Shell layout as the reference.

### 2.2 Resource References

- `inter.css` and `jetbrains-mono.css` must be referenced locally (no Google Fonts).
- `prototype-base.css` must be loaded either via `<link>` or inlined in a `<style>` block by the build pipeline. Legacy `tailwind-utilities.css` is still accepted.
- No forbidden CDN hostnames (`fonts.googleapis.com`, `fonts.gstatic.com`, `cdn.jsdelivr.net`, `unpkg.com`, `cdnjs.cloudflare.com`).

### 2.3 CSS Tokens

- A `:root` CSS variables block must exist.
- Required tokens must be defined: `--background`, `--foreground`, `--primary`, `--secondary`, `--muted`, `--border`, `--card`.
- Hardcoded hex/rgba colors are flagged as warnings, except for known semantic exceptions (`#ff3b30` danger buttons, icon SVGs). The accent bar should use `bg-primary/15`; any leftover `--accent-bar: #ca8a04` is a stale variable and should be removed from the shared utilities.

### 2.4 Prohibited Patterns (binary)

- No `gl-gateway-*` class names in HTML attributes or CSS selectors.
- No `react-router` dependency.
- No `scrollIntoView` usage (breaks container scroll).

### 2.5 Build Metadata (binary)

- The first-line HTML comment must declare the version: `Module m-vN`, `Block b-vN`, or `App a-vN`.
- The scene list must be resolved; `scenes: ?` indicates `buildModule` lost `sceneRefs` and is a build-tool bug.

### 2.6 Existing Gate Compliance

The evaluator delegates to the existing checks to avoid duplicating logic:

- `ontology-mapping prototype-check`: boot-skeleton, vendor paths, Babel/JSX safety, render try/catch, CDN
- `audit-css-framework.mjs`: framework reference, deprecated `gl-*`/`al-*` CSS rules, CSS syntax
- `check-class-names.js`: all used class names are defined (taken as a warning for generated ESM prototypes because Tailwind utilities can be noisy)

## 3. Reference Layout Contract (visual parity target)

The rendered DOM must match the following hierarchy, using Tailwind utility classes instead of the reference's `gl-gateway-*` classes:

```
#root
└── root (flex h-screen flex-col overflow-hidden bg-background)
    ├── header/topbar (h-14 border-b px-6 bg-background shrink-0)
    │   ├── left: logo/brand + breadcrumbs/module-tabs
    │   └── right: search + action group + user menu
    └── body (flex flex-1 min-h-0 overflow-hidden)
        ├── aside/navigation (w-60 ↔ w-16, bg-secondary, border-r)
        │   ├── nav/main-nav (sections + nav items)
        │   └── sidebar-foot (collapse button)
        ├── main (flex flex-col min-w-0 overflow-hidden flex-1)
        │   ├── accent-bar (h-[3px] w-full bg-primary/15)
        │   ├── content (flex-1 w-full h-full bg-muted/30 overflow-hidden)
        │   │   └── inner (flex flex-col h-full)
        │   │       ├── block-scroll (flex-1 min-h-0 overflow-y-auto)
        │   │       └── footer (hidden md:flex h-10 border-t bg-card)
        └── workspace-dock (w-80 border-l bg-card, conditional)
```

**Static limitation**: the generated HTML has an empty `#root` before React mounts. The evaluator verifies the shell is ready to produce this layout, but the actual rendered structure must be confirmed with `check-visual-verify.ts` + ego-browser screenshots.

## 4. Functional Features (verified via ego-browser, not statically)

- Sidebar navigation items render and are clickable.
- Clicking a nav item updates the active highlight and renders the corresponding Block/Scene content.
- Sidebar collapse button toggles width between expanded and collapsed.
- Theme toggle switches between light and dark mode (`document.documentElement.classList.toggle('dark')`).
- User menu opens a dropdown with user info, profile, settings, logout.
- Workspace dock toggles on/off when action buttons trigger it.
- Boot skeleton displays on load and fades out after React mount.
- Footer visible on `md+` breakpoints, hidden on small screens.

## 5. Iteration Workflow

```
Generate prototype (m-v{N}.html)
    │
    ▼
Run static evaluator → score + gap report
    │
    ▼
If static FAIL → fix source (module.tsx / block.tsx / prototype-tool.js) → rebuild → re-evaluate
    │
    ▼
If static PASS → run visual verify (check-visual-verify.ts) → screenshot → 6-dimension score
    │
    ▼
If visual FAIL → iterate design/source → rebuild → re-run visual verify
    │
    ▼
Deliver
```

## 6. Iteration Prompt Template

When a prototype fails evaluation, use this template to refine the source:

```markdown
You are refining the prototype {prototype-path} to match the Gateway Shell reference.

Current weighted score: {score}/100.
Failed dimensions: {dimensions}.
Specific gaps:
{gap-list}

Constraints:

- Keep using Tailwind utility classes and the shared `gateway-shell.tsx` shell.
- Do NOT introduce `gl-gateway-*` class names.
- Do NOT duplicate shell CSS.
- Fix only the listed gaps; do not change unrelated layout or business content.

Output the corrected `llm-tsx/module.tsx` (or `llm-tsx/block.tsx`) source code, then run `bun scripts/prototype-tool.js build <path>` and verify the evaluator passes.
```

## 7. Scoring Example

| Dimension                | Score | Notes                                            |
| ------------------------ | ----- | ------------------------------------------------ |
| Structural Shell         | 5     | Boot skeleton, root, vendor scripts present      |
| Resource References      | 5     | Local fonts, inline prototype-base utilities, no CDN   |
| CSS Tokens               | 4.7   | All tokens present, one stale `#ca8a04` variable |
| Prohibited Patterns      | 5     | No `gl-gateway-*`, no router-dom                 |
| Build Metadata           | 5     | Scene list resolved after `scenes: ?` fix        |
| Existing Gate Compliance | 5     | All existing checks pass                         |

Weighted = (5×0.25 + 5×0.15 + 4.7×0.20 + 5×0.15 + 5×0.05 + 5×0.20) = **98.8** → PASS.

## 8. Known Shared-Asset Issues

| Issue                                         | Location                 | Impact            | Fix                                                                             |
| --------------------------------------------- | ------------------------ | ----------------- | ------------------------------------------------------------------------------- |
| `--accent-bar: #ca8a04` stale hardcoded color | `prototype-base.css` (legacy `tailwind-utilities.css`) | CSS token warning | Remove variable; accent bar should use `bg-primary/15` from `gateway-shell.tsx` |

These issues are outside any single prototype and should be fixed in the shared references under `skill://alioth-design/references/`.
