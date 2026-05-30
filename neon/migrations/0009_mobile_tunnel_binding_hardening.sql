ALTER TABLE ctx.mobile_tunnel
  ADD COLUMN IF NOT EXISTS grant_id uuid,
  ADD COLUMN IF NOT EXISTS daemon_id text,
  ADD COLUMN IF NOT EXISTS device_id text;

UPDATE ctx.mobile_tunnel
SET daemon_id = user_id,
    device_id = user_id
WHERE status = 'active'
  AND disabled_at IS NULL
  AND (daemon_id IS NULL OR device_id IS NULL);

CREATE INDEX IF NOT EXISTS mobile_tunnel_binding_active_idx
  ON ctx.mobile_tunnel (user_id, daemon_id, device_id, status, disabled_at, created_at);

CREATE INDEX IF NOT EXISTS mobile_tunnel_grant_active_idx
  ON ctx.mobile_tunnel (grant_id, status, disabled_at, created_at)
  WHERE grant_id IS NOT NULL;
