# dsh-alioth — the complete AppAgent pipeline as a deployable container.
#
# The image ships the full Alioth consumer plugin group: PostgreSQL 18 (PGDG build,
# started by the entry script on /data — env-alioth connects through
# ALIOTH_DATABASE_URL and never provisions a cluster of its own),
# an assembled model source given as ALIOTH_MODEL_SOURCE (`builtin` was retired 2026-09-24;
# the package ships no snapshot). The keyless --check assembles a fixture source itself.
# the file-based semantic dictionaries, bun (prototype gates), and the dsh web
# entry point. Model-visible behavior: `dsh --profile web --patch <bundle>`.
#
# Harness sourcing (2026-09-04): @deepseek-ai devDependencies resolve to the
# deepseek-harness source tree (Alioth line, pinned in docker.yml). The build context MUST
# contain a sibling `deepseek-harness/` checkout (docker.yml provides it; local
# builds: clone it next to this repo and build from the parent directory, or
# pass the harness as an additional context).
#
# Build (CI layout): docker build -f dsh-alioth/Dockerfile .
# Run:    docker run --rm -p 3100:3100 -e DEEPSEEK_API_KEY=... -v alioth-data:/data dsh-alioth
# Self-check (keyless): docker run --rm dsh-alioth --check   # entry starts PG, then runs docker-check.sh

# ── build stage: install the workspace with production binaries ──
FROM node:24.20-slim AS build
# node-pty/koffi compile native bits when prebuilds are missing (linux-arm64).
RUN apt-get update && apt-get install -y --no-install-recommends python3 make g++ git \
  && rm -rf /var/lib/apt/lists/*
RUN corepack enable && corepack prepare pnpm@12.3.4 --activate

# Host harness source tree first: this workspace's @deepseek-ai devDeps
# resolve through ../deepseek-harness, and the harness must be built before
# /app's install links against its lib output.
WORKDIR /deepseek-harness
# The whole source tree, not an enumerated subset: the harness host build
# type-checks files across packages/, apps/, vendor/, native/, scripts/ and the
# root configs, so any missing directory surfaces as a build failure one at a
# time. .git/node_modules are excluded by the context .dockerignore the workflow
# writes (and by the sibling layout docker.yml checks out).
COPY deepseek-harness/ ./
# Not frozen, same reason as CI: the harness workspace lists out-of-root
# members (../dsh-chess, ../../.dsh-chess/profiles) whose importers are in its
# lockfile but cannot be checked out here. Lifecycle scripts are skipped: they
# install git hooks and runtime helpers (lefthook, spawn-helper) that this
# build-only stage never uses, and the first of them exits without git.
RUN pnpm install --no-frozen-lockfile --ignore-scripts
# AppCreator client patches (session pick gate, namespace isolation, picker
# controls): replayed from the consumer workspace before building.
RUN pnpm run build:lib:host && pnpm run build:lib:client

# ── the consumer workspace ──
# The build context is the PARENT of both checkouts (see the header), so the
# consumer sources carry the same dsh-alioth/ prefix as the harness sources
# above — without it the build fails on missing COPY sources.
WORKDIR /app
COPY dsh-alioth/package.json dsh-alioth/pnpm-workspace.yaml dsh-alioth/pnpm-lock.yaml ./
COPY dsh-alioth/packages ./packages
COPY dsh-alioth/examples ./examples
COPY dsh-alioth/scripts ./scripts
COPY dsh-alioth/tsconfig*.json ./
# onnxruntime-node / sharp run their native postinstall steps; allowBuilds in
# pnpm-workspace.yaml already whitelists them.
RUN pnpm install --frozen-lockfile
# Native deps must be present for the runtime stage without the toolchain.
#
# STALE / DEAD (noted 2026-09-22, left in place deliberately): the runtime stage
# below never copies /app/runtime-* — it copies the whole /app/packages and
# /app/node_modules trees, so these six filters produce nothing the image uses.
# Their package list is also frozen at the pre-2026-09-22 set and does not name
# guard-alioth / tool-alioth-verify / verify-alioth. Removing them is a
# behaviour-adjacent change to another author's build steps, so it is flagged
# here rather than done: either delete the block or point it at the packages
# that actually need their native deps materialised.
RUN pnpm --filter '@dsh-alioth/env-alioth' deploy --legacy --prod /app/runtime-env \
  && pnpm --filter '@dsh-alioth/tool-alioth' deploy --legacy --prod /app/runtime-tool \
  && pnpm --filter '@dsh-alioth/tool-alioth-meta' deploy --legacy --prod /app/runtime-meta \
  && pnpm --filter '@dsh-alioth/tool-alioth-workflow' deploy --legacy --prod /app/runtime-workflow \
  && pnpm --filter '@dsh-alioth/tool-alioth-orchestrator' deploy --legacy --prod /app/runtime-orchestrator \
  && pnpm --filter '@dsh-alioth/bundle-alioth' deploy --legacy --prod /app/runtime-bundle

# ── runtime stage: slim, no toolchain ──
FROM node:24.20-slim AS runtime
# bun — the declared prototype-gate runtime (distribution dependency).
# bun pinned to the AliothStudio stack version (prototype gates must match).
RUN npm install -g bun@1.4.2 --silent
# PostgreSQL 18.6 via PGDG — aligned with the AliothStudio stack (Homebrew 18.6)
# and with the host deployments. This is THE database the container uses:
# env-alioth takes it through ALIOTH_DATABASE_URL (set by docker-entry.sh).
RUN apt-get update && apt-get install -y --no-install-recommends locales curl ca-certificates gnupg \
  && sed -i 's/# en_US.UTF-8/en_US.UTF-8/' /etc/locale.gen && locale-gen en_US.UTF-8 \
  && curl -fsSL https://www.postgresql.org/media/keys/ACCC4CF8.asc | gpg --dearmor -o /usr/share/keyrings/pgdg.gpg \
  && echo "deb [signed-by=/usr/share/keyrings/pgdg.gpg] http://apt.postgresql.org/pub/repos/apt bookworm-pgdg main" > /etc/apt/sources.list.d/pgdg.list \
  && apt-get update && apt-get install -y --no-install-recommends postgresql-18 \
  && rm -rf /var/lib/apt/lists/*
WORKDIR /app

# Workspace sources (tsx runs .ts directly; strip-only compatible). The
# node_modules tree links @deepseek-ai/* into the harness checkout, so the
# harness package directories (lib output + manifests, no node_modules —
# their dependencies resolve through /app/node_modules/.pnpm) must ship too.
COPY --from=build /app/package.json /app/pnpm-workspace.yaml /app/
COPY --from=build /app/pnpm-lock.yaml /app/
COPY --from=build /app/node_modules /app/node_modules
COPY --from=build /app/packages /app/packages
COPY --from=build /app/scripts /app/scripts
COPY --from=build /app/examples /app/examples
COPY --from=build /app/tsconfig*.json /app/
COPY --from=build /deepseek-harness/packages /deepseek-harness/packages
COPY --from=build /deepseek-harness/vendor /deepseek-harness/vendor
COPY --from=build /deepseek-harness/apps /deepseek-harness/apps
COPY --from=build /deepseek-harness/native /deepseek-harness/native
COPY --from=build /deepseek-harness/package.json /deepseek-harness/package.json

# Web GUI port.
ENV DSH_WEB_PORT=3100 \
    DSH_OPEN=false \
    ALIOTH_DATA_ROOT=/data/alioth \
    PATH="/usr/lib/postgresql/18/bin:${PATH}" \
    PGPASSWORD=alioth

# Entry/check scripts (root-owned, world-readable, executable) and the data
# volume, then drop to the non-root node user — Postgres refuses to run as root,
# and the cluster lives under /data (metadata-only chown, no data copy).
COPY dsh-alioth/scripts/docker-entry.sh dsh-alioth/scripts/docker-check.sh /app/scripts/
RUN chmod +x /app/scripts/docker-entry.sh /app/scripts/docker-check.sh \
  && chown -R node:node /app \
  && mkdir -p /data && chown node:node /data
VOLUME ["/data"]
EXPOSE 3100 3900
USER node

ENTRYPOINT ["/app/scripts/docker-entry.sh"]
