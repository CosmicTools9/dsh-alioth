-- 014_create_audit_outbox.sql
-- 审计写入机制（ADR D-010）：Rust 层 transactional outbox 队列表。
-- 业务写事务内随源数据插入一行（同事务、零丢失）；outbox worker 独立事务
-- 批量转写到 isahl_audit.data_change_logs，失败退避重试，支持 dead 死信与重放。
-- 通道合规：isahl_audit 不冻结（CONTAINER_BOUNDARY §44/§46），本文件经
-- Gateway/backend/migrations/ 由 namespace_schema runner 应用。

CREATE TABLE IF NOT EXISTS isahl_audit.audit_outbox (
    id                  BIGINT      NOT NULL DEFAULT isahl.gen_next_zuid(),
    -- 差异流载荷（对齐 data_change_logs 列子集）
    table_schema        TEXT        NOT NULL DEFAULT 'isahl',
    table_name          TEXT        NOT NULL,
    record_id           BIGINT      NOT NULL,
    action              TEXT        NOT NULL,
    action_timestamp    TIMESTAMPTZ NOT NULL DEFAULT now(),  -- 业务事务发生时刻；worker 转写时透传，保物理时间锚
    changed_fields      JSONB,
    old_values          JSONB,
    new_values          JSONB,
    performed_by_id     BIGINT,
    performed_by_email  TEXT,
    transaction_id      TEXT,        -- 请求作用域生成贯穿；同事务多写共享同一 ID 成组
    session_id          TEXT,
    client_ip           INET,
    user_agent          TEXT,
    request_path        TEXT,
    request_method      TEXT,
    context             JSONB,
    -- outbox 状态机
    status              TEXT        NOT NULL DEFAULT 'pending',
    attempts            INTEGER     NOT NULL DEFAULT 0,
    next_retry_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_error          TEXT,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    processed_at        TIMESTAMPTZ,
    CONSTRAINT audit_outbox_pkey PRIMARY KEY (id),
    CONSTRAINT audit_outbox_status_check CHECK (status IN ('pending', 'processing', 'done', 'failed', 'dead')),
    CONSTRAINT audit_outbox_action_check CHECK (action IN ('INSERT', 'UPDATE', 'DELETE'))
);

-- worker 竞争领取（SKIP LOCKED）主查询路径
CREATE INDEX IF NOT EXISTS idx_audit_outbox_claim
    ON isahl_audit.audit_outbox (status, next_retry_at)
    WHERE status IN ('pending', 'failed');

-- as-of 合成读 pending 补偿 / 记录维度查询
CREATE INDEX IF NOT EXISTS idx_audit_outbox_record
    ON isahl_audit.audit_outbox (table_schema, table_name, record_id);

-- 事务成组查询
CREATE INDEX IF NOT EXISTS idx_audit_outbox_tx
    ON isahl_audit.audit_outbox (transaction_id);

-- 滞后观测（worker 落后时长 = now() - created_at where status='pending'）
CREATE INDEX IF NOT EXISTS idx_audit_outbox_created
    ON isahl_audit.audit_outbox (created_at);
