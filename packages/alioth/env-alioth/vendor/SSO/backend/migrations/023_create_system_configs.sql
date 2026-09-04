-- 013_create_system_configs.sql
-- 系统配置表（基础设施接入：LLM / Email / IM / Webhook / Storage / SMS）
--
-- 归属：isahl_auth schema（Gateway / SSO / AppCreator 均可读写；isahl schema 自
-- v10.x 冻结禁止建表，isahl_meta 为 Gateway 禁区，故配置表置于 isahl_auth）。
-- ID：与 isahl_auth 其它表一致使用 isahl.gen_next_zuid()。
-- 敏感凭证（credentials）由应用层 AES-256-GCM 加密后存储。

CREATE TABLE IF NOT EXISTS isahl_auth.system_configs (
    id            bigint PRIMARY KEY DEFAULT isahl.gen_next_zuid(),
    notice        text,
    code          text,
    "_f_"         text,
    "_t_"         text,
    comments      text,
    credentials   jsonb,
    settings      jsonb,
    enabled       boolean NOT NULL DEFAULT true,
    is_default    boolean NOT NULL DEFAULT false,
    domain_       text,
    public        boolean NOT NULL DEFAULT false,
    created_at    timestamptz NOT NULL DEFAULT now(),
    updated_at    timestamptz NOT NULL DEFAULT now(),
    created_by_id bigint,
    updated_by_id bigint,
    deleted_at    timestamptz
);

COMMENT ON TABLE isahl_auth.system_configs IS
  '系统配置（基础设施接入：LLM/Email/IM/Webhook/Storage/SMS），敏感凭证 AES-256-GCM 加密存储';
