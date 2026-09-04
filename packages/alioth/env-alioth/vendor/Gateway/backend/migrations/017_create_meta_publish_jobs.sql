-- 017_create_isahl_meta.meta_publish_jobs.sql
-- Publish job state machine for app-instance async publishing.
-- 4-step pipeline: validate -> sync-prototype -> update-status -> gateway-restart

CREATE TABLE IF NOT EXISTS isahl_meta.meta_publish_jobs (
    id              BIGSERIAL PRIMARY KEY,
    fk_app_instance BIGINT      NOT NULL,
    stage           TEXT        NOT NULL DEFAULT 'generating',
    job_state       TEXT        NOT NULL DEFAULT 'pending',
    current_step    TEXT,
    steps           JSONB       NOT NULL DEFAULT '[]'::JSONB,
    error_detail    TEXT,
    created_by_id   BIGINT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    deleted_at      TIMESTAMPTZ,

    CONSTRAINT chk_job_state CHECK (job_state IN ('pending', 'running', 'success', 'failed', 'cancelled', 'aborted'))
);

CREATE INDEX idx_meta_publish_jobs_instance ON isahl_meta.meta_publish_jobs(fk_app_instance) WHERE deleted_at IS NULL;
CREATE INDEX idx_meta_publish_jobs_state   ON isahl_meta.meta_publish_jobs(job_state) WHERE deleted_at IS NULL;

COMMENT ON TABLE  isahl_meta.meta_publish_jobs IS 'App-instance publish job records -- 4-step async pipeline';
COMMENT ON COLUMN isahl_meta.meta_publish_jobs.steps IS 'Array of StepState: {step,state,started_at,finished_at,error}';

-- 与 isahl_meta 既有表（meta_mise_tasks）保持一致的授权面：
-- 本地 dev/test 环境应用与测试角色需要完整 DML + 序列 USAGE
GRANT SELECT, INSERT, UPDATE, DELETE ON isahl_meta.meta_publish_jobs TO alioth_readonly;
GRANT USAGE ON SEQUENCE isahl_meta.meta_publish_jobs_id_seq TO alioth_readonly;
GRANT SELECT, INSERT, UPDATE, DELETE ON isahl_meta.meta_publish_jobs TO "william.d.zk";
GRANT USAGE ON SEQUENCE isahl_meta.meta_publish_jobs_id_seq TO "william.d.zk";
