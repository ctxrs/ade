.PHONY: test dev web supabase-local supabase-functions dev-preflight

CARGO_TARGET_DIR ?= $(HOME)/.cache/cargo/ctx-monorepo
CTX_MCP_COMMAND ?= $(CARGO_TARGET_DIR)/debug/ctx-mcp
CTX_DAEMON_PORT ?= 4399
CTX_DAEMON_BIND ?= 127.0.0.1:$(CTX_DAEMON_PORT)
CTX_DATA_DIR ?=
CTX_DAEMON_UNIT ?= ctx-daemon-$(CTX_DAEMON_PORT)
CTX_DAEMON_STOP ?= 0
CTX_DAEMON_DATA_DIR ?= $(if $(CTX_DATA_DIR),$(CTX_DATA_DIR),$(HOME)/.ctx)
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

dev: dev-preflight supabase-local supabase-functions
	cargo build -p ctx-mcp
ifeq ($(shell uname -s),Linux)
	@if command -v systemd-run >/dev/null 2>&1 && systemctl --user show-environment >/dev/null 2>&1; then \
		if [ "$(CTX_DAEMON_STOP)" = "1" ]; then \
			systemctl --user stop $(CTX_DAEMON_UNIT).scope >/dev/null 2>&1 || true; \
			systemctl --user reset-failed $(CTX_DAEMON_UNIT).scope >/dev/null 2>&1 || true; \
		fi; \
		systemd-run --user --scope --unit $(CTX_DAEMON_UNIT) --same-dir --collect \
			--setenv CTX_MCP_COMMAND=$(CTX_MCP_COMMAND) \
			cargo run -p ctx-http --bin ctx -- serve --bind $(CTX_DAEMON_BIND) $(if $(CTX_DATA_DIR),--data-dir $(CTX_DATA_DIR),); \
	else \
		CTX_MCP_COMMAND=$(CTX_MCP_COMMAND) cargo run -p ctx-http --bin ctx -- serve --bind $(CTX_DAEMON_BIND) $(if $(CTX_DATA_DIR),--data-dir $(CTX_DATA_DIR),); \
	fi
else
	CTX_MCP_COMMAND=$(CTX_MCP_COMMAND) cargo run -p ctx-http --bin ctx -- serve --bind $(CTX_DAEMON_BIND) $(if $(CTX_DATA_DIR),--data-dir $(CTX_DATA_DIR),)
endif

dev-preflight:
	@if [ "$(CTX_DAEMON_STOP)" = "1" ]; then \
		exit 0; \
	fi; \
	port="$(CTX_DAEMON_PORT)"; \
	data_dir="$(CTX_DAEMON_DATA_DIR)"; \
	port_in_use=0; \
	if command -v lsof >/dev/null 2>&1; then \
		if lsof -nP -iTCP:"$$port" -sTCP:LISTEN >/dev/null 2>&1; then port_in_use=1; fi; \
	elif command -v ss >/dev/null 2>&1; then \
		if ss -ltn "sport = :$$port" | tail -n +2 | grep -q .; then port_in_use=1; fi; \
	elif command -v netstat >/dev/null 2>&1; then \
		if netstat -ltn 2>/dev/null | awk '{print $$4}' | grep -q ":$$port$$"; then port_in_use=1; fi; \
	fi; \
	lock_path="$$data_dir/daemon.lock"; \
	lock_active=0; \
	lock_pid=""; \
	if [ -e "$$lock_path" ]; then \
		lock_pid="$$(tr -d '[:space:]' < "$$lock_path" 2>/dev/null || true)"; \
		if echo "$$lock_pid" | grep -Eq '^[0-9]+$$'; then \
			if ps -p "$$lock_pid" >/dev/null 2>&1; then \
				cmd="$$(ps -p "$$lock_pid" -o command= 2>/dev/null || true)"; \
				if echo "$$cmd" | grep -q 'ctx .*serve'; then lock_active=1; fi; \
			fi; \
		fi; \
		if [ "$$lock_active" != "1" ] && [ "$$port_in_use" = "0" ]; then \
			rm -f "$$lock_path"; \
		fi; \
	fi; \
	if [ "$$port_in_use" = "1" ] || [ "$$lock_active" = "1" ]; then \
		echo "error: ctx daemon preflight failed." >&2; \
		if [ "$$port_in_use" = "1" ]; then \
			echo "  Port $$port is already in use." >&2; \
		fi; \
		if [ "$$lock_active" = "1" ]; then \
			echo "  Data dir appears in use: $$data_dir (daemon pid $$lock_pid)." >&2; \
		fi; \
		echo "hint: run another daemon with a different port + data dir:" >&2; \
		echo "  CTX_DAEMON_PORT=4401 CTX_DATA_DIR=/tmp/ctx-dev-4401 make dev" >&2; \
		echo "hint: or set CTX_DAEMON_STOP=1 to stop the existing daemon first." >&2; \
		exit 1; \
	fi

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
