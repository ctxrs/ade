CREATE TABLE IF NOT EXISTS mobile_tunnels (
    tunnel_id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    relay_base_url TEXT NOT NULL,
    public_base_url TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    disabled_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_mobile_tunnels_user_id ON mobile_tunnels(user_id);
