ALTER TABLE ctx.billing_spend_limits
  ADD COLUMN IF NOT EXISTS status text NOT NULL DEFAULT 'active',
  ADD COLUMN IF NOT EXISTS period_start timestamptz,
  ADD COLUMN IF NOT EXISTS period_end timestamptz,
  ADD COLUMN IF NOT EXISTS hard_limit_cents integer;

UPDATE ctx.billing_spend_limits
SET status = CASE WHEN active THEN 'active' ELSE 'disabled' END
WHERE active IS NOT NULL;

UPDATE ctx.billing_spend_limits
SET hard_limit_cents = limit_cents
WHERE hard_limit_cents IS NULL;

UPDATE ctx.billing_spend_limits
SET period_start = CASE
    WHEN period = 'day' THEN date_trunc('day', now())
    WHEN period = 'month' THEN date_trunc('month', now())
    ELSE '1970-01-01 00:00:00+00'::timestamptz
  END
WHERE period_start IS NULL;

UPDATE ctx.billing_spend_limits
SET period_end = CASE
    WHEN period = 'day' THEN date_trunc('day', now()) + interval '1 day'
    WHEN period = 'month' THEN date_trunc('month', now()) + interval '1 month'
    ELSE 'infinity'::timestamptz
  END
WHERE period_end IS NULL;

ALTER TABLE ctx.billing_spend_limits
  ALTER COLUMN status SET NOT NULL,
  ALTER COLUMN period_start SET NOT NULL,
  ALTER COLUMN period_end SET NOT NULL,
  ALTER COLUMN hard_limit_cents SET NOT NULL;

DO $$
BEGIN
  IF NOT EXISTS (
    SELECT 1
    FROM pg_constraint
    WHERE conname = 'billing_spend_limits_status_check'
      AND conrelid = 'ctx.billing_spend_limits'::regclass
  ) THEN
    ALTER TABLE ctx.billing_spend_limits
      ADD CONSTRAINT billing_spend_limits_status_check
      CHECK (status IN ('active', 'disabled')) NOT VALID;
  END IF;
END
$$;

ALTER TABLE ctx.billing_spend_limits
  VALIDATE CONSTRAINT billing_spend_limits_status_check;

DO $$
BEGIN
  IF NOT EXISTS (
    SELECT 1
    FROM pg_constraint
    WHERE conname = 'billing_spend_limits_period_bounds_check'
      AND conrelid = 'ctx.billing_spend_limits'::regclass
  ) THEN
    ALTER TABLE ctx.billing_spend_limits
      ADD CONSTRAINT billing_spend_limits_period_bounds_check
      CHECK (period_end > period_start) NOT VALID;
  END IF;
END
$$;

ALTER TABLE ctx.billing_spend_limits
  VALIDATE CONSTRAINT billing_spend_limits_period_bounds_check;

DO $$
BEGIN
  IF NOT EXISTS (
    SELECT 1
    FROM pg_constraint
    WHERE conname = 'billing_spend_limits_hard_limit_cents_check'
      AND conrelid = 'ctx.billing_spend_limits'::regclass
  ) THEN
    ALTER TABLE ctx.billing_spend_limits
      ADD CONSTRAINT billing_spend_limits_hard_limit_cents_check
      CHECK (hard_limit_cents >= 0) NOT VALID;
  END IF;
END
$$;

ALTER TABLE ctx.billing_spend_limits
  VALIDATE CONSTRAINT billing_spend_limits_hard_limit_cents_check;

CREATE INDEX IF NOT EXISTS billing_spend_limits_active_period_idx
  ON ctx.billing_spend_limits (billing_subject_id, status, period_start, period_end, ctx_user_id);

REVOKE INSERT, UPDATE ON ctx.mobile_tunnel_grants FROM ctx_mobile_tunnel;
GRANT SELECT ON ctx.mobile_tunnel_grants TO ctx_mobile_tunnel;
GRANT SELECT, INSERT, UPDATE ON
  ctx.mobile_tunnel_sessions,
  ctx.mobile_tunnel_state_events,
  ctx.mobile_tunnel_revocations
TO ctx_mobile_tunnel;
