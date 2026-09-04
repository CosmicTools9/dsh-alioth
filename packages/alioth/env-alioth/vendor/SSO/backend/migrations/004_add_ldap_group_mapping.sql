ALTER TABLE isahl_auth.ldap_configs
  ADD COLUMN IF NOT EXISTS group_mapping JSONB DEFAULT '{}'::jsonb;
COMMENT ON COLUMN isahl_auth.ldap_configs.group_mapping IS 'LDAP group DN to NGAC UA o_name mapping, e.g. {"cn=admins,dc=corp":"admin","cn=users,dc=corp":"user"}';
