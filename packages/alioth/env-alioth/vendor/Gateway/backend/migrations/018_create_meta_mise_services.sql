-- Migration 018: meta_mise_services（dev 服务登记表）
-- Change: add-mise-dev-services（design.md D-1）
-- Schema: isahl_meta（唯一入口 Gateway/backend/migrations/，ENVIRONMENT_SPEC §7.7）

CREATE TABLE IF NOT EXISTS isahl_meta.meta_mise_services (
    id              BIGSERIAL PRIMARY KEY,
    name            TEXT        NOT NULL,            -- 服务显示名
    kind            TEXT        NOT NULL,            -- 'mise_task' | 'manual'
    command         TEXT,                           -- 启动命令（mise_task 时为 task 名）
    cwd             TEXT,                           -- 工作目录
    port            INTEGER,                        -- 监听端口（可为 NULL）
    run_token       TEXT,                           -- 启动时生成的 UUID，唯一身份
    last_pid        INTEGER,                        -- 最近一次运行的 PID
    last_pgid       INTEGER,                        -- 最近一次运行的进程组 PGID
    last_uid        BIGINT,                         -- 启动时的 UID
    status          TEXT        NOT NULL DEFAULT 'stopped',  -- 'running' | 'stopped' | 'failed'
    auto_restart    BOOLEAN     NOT NULL DEFAULT false,
    created_by_id   BIGINT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    deleted_at      TIMESTAMPTZ,
    CONSTRAINT valid_kind CHECK (kind IN ('mise_task', 'manual'))
);

CREATE INDEX IF NOT EXISTS idx_meta_mise_services_status ON isahl_meta.meta_mise_services(status);
CREATE INDEX IF NOT EXISTS idx_meta_mise_services_port ON isahl_meta.meta_mise_services(port) WHERE port IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_meta_mise_services_run_token ON isahl_meta.meta_mise_services(run_token);
