# dsh-alioth — the complete AppAgent pipeline as a deployable container.
#
# The image ships the full Alioth consumer plugin group: embedded PostgreSQL 18,
# the frozen builtin model (vendor artifacts, zero network at first boot),
# the file-based semantic dictionaries, bun (prototype gates), and the dsh web
# entry point. Model-visible behavior: `dsh --profile web --patch <bundle>`.
#
# Build:  docker build -t dsh-alioth .
# Run:    docker run --rm -p 3100:3100 -e DEEPSEEK_API_KEY=... -v alioth-data:/data dsh-alioth
# Self-check (keyless): docker run --rm --entrypoint /app/scripts/docker-check.sh dsh-alioth

# ── build stage: install the workspace with production binaries ──
FROM node:24.19-slim AS build
# node-pty/koffi compile native bits when prebuilds are missing (linux-arm64).
RUN apt-get update && apt-get install -y --no-install-recommends python3 make g++ \
  && rm -rf /var/lib/apt/lists/*
RUN corepack enable && corepack prepare pnpm@11.22.0 --activate
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
FROM node:24.19-slim AS runtime
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

# Workspace sources (tsx runs .ts directly; strip-only compatible).
COPY --from=build /app/package.json /app/pnpm-workspace.yaml /app/
COPY --from=build /app/pnpm-lock.yaml /app/
COPY --from=build /app/node_modules /app/node_modules
COPY --from=build /app/packages /app/packages
COPY --from=build /app/scripts /app/scripts
COPY --from=build /app/examples /app/examples
COPY --from=build /app/tsconfig*.json /app/

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
