-- 为已应用 014 的数据库补加级联删除：删除用户时自动清理其 WebAuthn 凭据与挑战。
-- 全新安装（014 已含 ON DELETE CASCADE）执行本迁移为幂等空操作。

DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM information_schema.table_constraints
        WHERE constraint_name = 'webauthn_credentials_user_id_fkey'
          AND table_schema = 'isahl_auth'
          AND table_name = 'webauthn_credentials'
    ) THEN
        ALTER TABLE isahl_auth.webauthn_credentials
            DROP CONSTRAINT webauthn_credentials_user_id_fkey;
        ALTER TABLE isahl_auth.webauthn_credentials
            ADD CONSTRAINT webauthn_credentials_user_id_fkey
                FOREIGN KEY (user_id)
                REFERENCES isahl_auth.auth_users(id)
                ON DELETE CASCADE;
    END IF;
END $$;
