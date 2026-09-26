# Third-party notices

Aggregate disclosure for what dsh-alioth ships. Component-level files travel with
the artifacts themselves: the container image carries `/app/LICENSE`,
`/app/NOTICE`, this file, and the DeepSeek Harness license + disclosure (see
"Bundled runtime"); the vendored model artifacts carry their own `LICENSE` and
`NOTICE` inside `packages/alioth/env-alioth/vendor/`.

## dsh-alioth itself

Apache License 2.0 — see `LICENSE`. Copyright 2026 the dsh-alioth authors.
Attribution detail (vendored artifacts, generated dictionaries) is in `NOTICE`.

## DeepSeek Harness (`@deepseek-ai/*`) — MIT

dsh-alioth is a plugin group for the DeepSeek Harness: it depends on harness
packages (`@deepseek-ai/cordis`, `@deepseek-ai/dsh-*`), and the container image
this repository builds bundles a built harness checkout.

- License: MIT — Copyright (c) 2026 DeepSeek
- Upstream: <https://github.com/deepseek-ai/deepseek-harness>
- Notices: the harness's own `LICENSE` and `THIRD_PARTY_NOTICES.md` (its
  transitive-dependency disclosure) are copied **verbatim** into the image at
  `/deepseek-harness/`.

## Alioth model artifacts — MIT / Apache-2.0

- The generated semantic dictionaries
  (`packages/alioth/skill-alioth/src/data/*.json`) derive from the Alioth model
  repository (<https://github.com/CosmicTools9/Alioth>) — MIT License,
  Copyright (c) 2025 宇器科技.
- The vendored consumption-side artifacts under
  `packages/alioth/env-alioth/vendor/` are Apache-2.0 works of The Alioth
  Authors (historical AppCreator open distribution), redistributed with their
  in-tree `vendor/LICENSE` + `vendor/NOTICE`.

## Container runtime base

The image's runtime stage also redistributes, unmodified:

- the Debian base image (`node:24.20-slim`) and its packages, including
  PostgreSQL 18 from PGDG — each Debian package ships its own copyright/license
  file inside the image under `/usr/share/doc/<package>/copyright`;
- Node.js — MIT;
- bun 1.4.2 (installed from npm) — MIT.

## Commercial use

The licenses above are permissive: Apache-2.0 and MIT both grant commercial use,
so no further permission is required *by them*. Separately from those copyright
licenses, this project's operator requires **authorization before any commercial
use** of dsh-alioth (including offering it, or a service built on it, for a fee).
That authorization is a contractual matter with the copyright holders; it does
not alter the file-level licenses above, which stay accurate as declared.

---

The statements here describe this repository's own distribution and do not
modify any third-party license.
