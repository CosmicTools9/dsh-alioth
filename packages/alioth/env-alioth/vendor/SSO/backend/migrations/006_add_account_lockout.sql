-- ============================================================================
-- 006_add_account_lockout.sql
-- Account lockout support (SECURITY_SPEC §5): brute-force protection.
--   5 consecutive failures within the window → 15-minute lockout.
-- DDL gate: backup-ddl.sh executed before applying this migration.
-- ============================================================================

ALTER TABLE isahl_auth.auth_users
    ADD COLUMN IF NOT EXISTS failed_login_attempts INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS locked_until TIMESTAMPTZ NULL;

COMMENT ON COLUMN isahl_auth.auth_users.failed_login_attempts
    IS '连续登录失败次数；达到阈值 (5) 后锁定账户 (SECURITY_SPEC §5)';
COMMENT ON COLUMN isahl_auth.auth_users.locked_until
    IS '账户锁定截止时间；NULL 表示未锁定。登录成功后清零 (SECURITY_SPEC §5)';
