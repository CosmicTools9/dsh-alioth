-- 016: 为 auth_users 增加通知偏好 JSONB 列
-- 对应 handlers/auth/notification_preferences.rs 的读写目标列。
ALTER TABLE isahl_auth.auth_users
    ADD COLUMN IF NOT EXISTS notification_preferences JSONB NOT NULL DEFAULT '{}'::jsonb;

COMMENT ON COLUMN isahl_auth.auth_users.notification_preferences IS
    '用户通知偏好（审批/IM/邮件/日程/公告/免打扰时段/自定义），由 /auth/me/notification-preferences 读写';
