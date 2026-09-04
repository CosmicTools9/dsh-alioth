-- Add subscriptions JSONB column to auth_users for notification subscriptions
ALTER TABLE isahl_auth.auth_users
  ADD COLUMN IF NOT EXISTS subscriptions JSONB DEFAULT '[]'::jsonb NOT NULL;
