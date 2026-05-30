ALTER TABLE ctx.model_prices
  ADD COLUMN IF NOT EXISTS input_microusd_per_1k_tokens bigint NOT NULL DEFAULT 0,
  ADD COLUMN IF NOT EXISTS output_microusd_per_1k_tokens bigint NOT NULL DEFAULT 0;

UPDATE ctx.model_prices
SET input_microusd_per_1k_tokens = (input_microusd_per_token::bigint * 1000),
    output_microusd_per_1k_tokens = (output_microusd_per_token::bigint * 1000)
WHERE input_microusd_per_1k_tokens = 0
  AND output_microusd_per_1k_tokens = 0
  AND (input_microusd_per_token > 0 OR output_microusd_per_token > 0);

DO $$
BEGIN
  IF NOT EXISTS (
    SELECT 1
    FROM pg_constraint
    WHERE conname = 'model_prices_input_microusd_per_1k_tokens_check'
      AND conrelid = 'ctx.model_prices'::regclass
  ) THEN
    ALTER TABLE ctx.model_prices
      ADD CONSTRAINT model_prices_input_microusd_per_1k_tokens_check
      CHECK (input_microusd_per_1k_tokens >= 0) NOT VALID;
  END IF;
END
$$;

ALTER TABLE ctx.model_prices
  VALIDATE CONSTRAINT model_prices_input_microusd_per_1k_tokens_check;

DO $$
BEGIN
  IF NOT EXISTS (
    SELECT 1
    FROM pg_constraint
    WHERE conname = 'model_prices_output_microusd_per_1k_tokens_check'
      AND conrelid = 'ctx.model_prices'::regclass
  ) THEN
    ALTER TABLE ctx.model_prices
      ADD CONSTRAINT model_prices_output_microusd_per_1k_tokens_check
      CHECK (output_microusd_per_1k_tokens >= 0) NOT VALID;
  END IF;
END
$$;

ALTER TABLE ctx.model_prices
  VALIDATE CONSTRAINT model_prices_output_microusd_per_1k_tokens_check;
