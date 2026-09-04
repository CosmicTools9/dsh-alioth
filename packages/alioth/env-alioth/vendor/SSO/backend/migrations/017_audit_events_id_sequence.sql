-- 017: 为 audit_events.id 增加序列默认值，使 EPP EventHandler 的
-- record_access_event（不显式绑定 id）能够正常插入。
CREATE SEQUENCE IF NOT EXISTS isahl_audit.audit_events_id_seq
    MINVALUE 1 START WITH 1 INCREMENT BY 1;

ALTER TABLE isahl_audit.audit_events
    ALTER COLUMN id SET DEFAULT nextval('isahl_audit.audit_events_id_seq');
