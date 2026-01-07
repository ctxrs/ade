.PHONY: test dev web supabase-local supabase-functions

CARGO_TARGET_DIR ?= $(HOME)/.cache/cargo/ctx-monorepo
CTX_MCP_COMMAND ?= $(CARGO_TARGET_DIR)/debug/ctx-mcp
SUPABASE_LOCAL ?= 1
SUPABASE_FUNCTIONS ?= 1
SUPABASE_FUNCTIONS_BG ?= 1
SUPABASE_PROD ?= 0
CTX_WEB_PORT ?= 5173
export CARGO_TARGET_DIR

ifeq ($(SUPABASE_PROD),1)
  SUPABASE_LOCAL := 0
  SUPABASE_FUNCTIONS := 0
endif

test:
	cargo test -q
	pnpm -C apps/web test

supabase-local:
ifeq ($(SUPABASE_LOCAL),1)
	@../scripts/supabase_local_ensure.sh
endif

supabase-functions: supabase-local
ifeq ($(SUPABASE_FUNCTIONS),1)
	@../scripts/supabase_local_functions.sh $(if $(filter 1,$(SUPABASE_FUNCTIONS_BG)),--background,)
endif

dev: supabase-local supabase-functions
	cargo build -p ctx-mcp
ifeq ($(shell uname -s),Linux)
	@if command -v systemd-run >/dev/null 2>&1 && systemctl --user show-environment >/dev/null 2>&1; then \
		systemctl --user stop ctx-daemon.scope >/dev/null 2>&1 || true; \
		systemctl --user reset-failed ctx-daemon.scope >/dev/null 2>&1 || true; \
		systemd-run --user --scope --unit ctx-daemon --same-dir --collect \
			--setenv CTX_MCP_COMMAND=$(CTX_MCP_COMMAND) \
			cargo run -p ctx-http --bin ctx -- serve; \
	else \
		CTX_MCP_COMMAND=$(CTX_MCP_COMMAND) cargo run -p ctx-http --bin ctx -- serve; \
	fi
else
	CTX_MCP_COMMAND=$(CTX_MCP_COMMAND) cargo run -p ctx-http --bin ctx -- serve
endif

web: supabase-local supabase-functions
ifeq ($(SUPABASE_PROD),1)
	@SUPABASE_ENV="$$(../scripts/supabase_prod_env.sh)"; \
	if [ -z "$$SUPABASE_ENV" ]; then \
		echo "error: SUPABASE_PROD=1 but prod Supabase env is missing." >&2; \
		exit 1; \
	fi; \
	env CTX_WEB_PORT=$(CTX_WEB_PORT) $$SUPABASE_ENV pnpm -C apps/web dev
else
ifeq ($(SUPABASE_LOCAL),1)
	@SUPABASE_ENV="$$(../scripts/supabase_local_env.sh)"; \
	if [ -n "$$SUPABASE_ENV" ]; then \
		env CTX_WEB_PORT=$(CTX_WEB_PORT) $$SUPABASE_ENV pnpm -C apps/web dev; \
	else \
		CTX_WEB_PORT=$(CTX_WEB_PORT) pnpm -C apps/web dev; \
	fi
else
	CTX_WEB_PORT=$(CTX_WEB_PORT) pnpm -C apps/web dev
endif
endif
