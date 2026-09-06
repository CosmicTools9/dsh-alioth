# dsh-alioth — the complete AppAgent pipeline as a deployable container.
#
# The image ships the full Alioth consumer plugin group: embedded PostgreSQL 18,
# the frozen builtin model (vendor artifacts, zero network at first boot),
# the file-based semantic dictionaries, bun (prototype gates), and the dsh web
# entry point. Model-visible behavior: `dsh --profile web --patch <bundle>`.
#
# Harness sourcing (2026-09-04): @deepseek-ai devDependencies resolve to the
# deepseek-harness source tree (tag dsh-v0.1.3-alpha.1). The build context MUST
# contain a sibling `deepseek-harness/` checkout (docker.yml provides it; local
# builds: clone it next to this repo and build from the parent directory, or
# pass the harness as an additional context).
#
# Build (CI layout): docker build -f dsh-alioth/Dockerfile .
# Run:    docker run --rm -p 3100:3100 -e DEEPSEEK_API_KEY=... -v alioth-data:/data dsh-alioth
# Self-check (keyless): docker run --rm --entrypoint /app/scripts/docker-check.sh dsh-alioth

# ── build stage: install the workspace with production binaries ──
FROM node:24.20-slim AS build
# node-pty/koffi compile native bits when prebuilds are missing (linux-arm64).
RUN apt-get update && apt-get install -y --no-install-recommends python3 make g++ \
  && rm -rf /var/lib/apt/lists/*
RUN corepack enable && corepack prepare pnpm@11.24.0 --activate

# Host harness source tree first: this workspace's @deepseek-ai devDeps
# resolve through ../deepseek-harness, and the harness must be built before
# /app's install links against its lib output.
WORKDIR /deepseek-harness
COPY deepseek-harness/package.json deepseek-harness/pnpm-workspace.yaml deepseek-harness/pnpm-lock.yaml deepseek-harness/tsconfig*.json ./
COPY deepseek-harness/vendor ./vendor
COPY deepseek-harness/packages ./packages
COPY deepseek-harness/apps ./apps
COPY deepseek-harness/native ./native
COPY dsh-alioth/scripts/harness-patches /harness-patches
RUN pnpm install --frozen-lockfile
# AppCreator client patches (session pick gate, namespace isolation, picker
# controls): replayed from the consumer workspace before building.
RUN git apply /harness-patches/ui-workspace-pick-gate.patch
RUN pnpm run build:lib:host && pnpm run build:lib:client

# ── the consumer workspace ──
WORKDIR /app
COPY package.json pnpm-workspace.yaml pnpm-lock.yaml ./
COPY packages ./packages
COPY examples ./examples
COPY scripts ./scripts
COPY tsconfig*.json ./
# onnxruntime-node / embedded-postgres / sharp run their native postinstall
# steps; allowBuilds in pnpm-workspace.yaml already whitelists them.
RUN pnpm install --frozen-lockfile
# Native deps must be present for the runtime stage without the toolchain.
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
RUN npm install -g bun@1.3.14 --silent
# embedded-postgres hard-codes LC_MESSAGES=en_US.UTF-8 for initdb; Debian
# slim ships only C/POSIX — generate the locale or PG init fails.
# PostgreSQL 18.6 via PGDG — aligned with the AliothStudio stack (Homebrew
# 18.6); embedded-postgres (npm) tops out at 18.4, so the container runs the
# official build and env-alioth connects through ALIOTH_DATABASE_URL.
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
    ALIOTH_MODEL_SOURCE=builtin \
    PATH="/usr/lib/postgresql/18/bin:${PATH}" \
    PGPASSWORD=alioth

# Entry/check scripts (root-owned, world-readable, executable) and the data
# volume, then drop to the non-root node user — Postgres refuses root, and
# embedded-postgres chmods its PG binary at startup (needs write access to
# node_modules; chown is metadata-only, no data copy).
COPY scripts/docker-entry.sh scripts/docker-check.sh /app/scripts/
RUN chmod +x /app/scripts/docker-entry.sh /app/scripts/docker-check.sh \
  && chown -R node:node /app \
  && mkdir -p /data && chown node:node /data
VOLUME ["/data"]
EXPOSE 3100 3900
USER node

ENTRYPOINT ["/app/scripts/docker-entry.sh"]
