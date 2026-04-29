.PHONY: test dev dev-codex-crp-local web supabase-local supabase-functions dev-preflight

ifeq ($(origin CTX_MAKE_SESSION_ID), undefined)
CTX_MAKE_SESSION_ID := make-$(shell date +%s%N 2>/dev/null || date +%s)
else
CTX_MAKE_SESSION_ID := $(CTX_MAKE_SESSION_ID)
endif
CTX_CACHE_EXPORT_CMD = export CTX_SESSION_ID="$(CTX_MAKE_SESSION_ID)"; export CTX_CACHE_SCOPE_KEY="$(CTX_MAKE_SESSION_ID)"; eval "$$(node scripts/print_ctx_cache_env.cjs --mode workspace --cwd "$(CURDIR)" --format shell --mkdir)"
CTX_CACHE_RUN = node scripts/run_with_ctx_cache_env.cjs --mode workspace --cwd "$(CURDIR)" --
CTX_MCP_COMMAND ?=
CTX_CRP_PROFILE ?= debug
CTX_INSTALL_LOCAL_CODEX_CRP ?= 0
CTX_DEV_MODE ?= 1
CTX_DAEMON_PORT ?= 4399
CTX_DAEMON_BIND ?= 127.0.0.1:$(CTX_DAEMON_PORT)
CTX_DATA_DIR ?=
CTX_DAEMON_UNIT ?= ctx-daemon-$(CTX_DAEMON_PORT)
CTX_DAEMON_STOP ?= 0
CTX_DAEMON_FEATURES ?=
CTX_DAEMON_MALLOC_CONF ?=
CTX_DAEMON_DATA_DIR ?= $(if $(CTX_DATA_DIR),$(CTX_DATA_DIR),$(HOME)/.ctx)
SUPABASE_LOCAL ?= 1
SUPABASE_FUNCTIONS ?= 1
SUPABASE_FUNCTIONS_BG ?= 1
SUPABASE_PROD ?= 0
CTX_WEB_PORT ?= 5173
DEV_LOG ?= 1
DEV_RUST_LOG ?= info
DEV_BACKTRACE ?= 1
DEV_LOG_STDOUT ?= 1
DEV_LOG_STDOUT_FILTER ?= error
DEV_LOG_MAX_BYTES ?= 0
DEV_DEBUG ?= 1
DEV_ACP_HEAP_PROFILE ?= 1
DEV_ACP_HEAP_PROFILE_LG_INTERVAL ?= 30
DEV_ACP_HEAP_PROFILE_LG_SAMPLE ?= 19
DEV_RESOURCE_TELEMETRY_INTERVAL_MS ?= 5000
DEV_RESOURCE_TELEMETRY_LOCAL_MAX_BYTES ?= 209715200
DEV_ENABLE_CODEX_CRP ?= 1
DEV_MODE ?= 1

DEV_LOG_ENV := $(if $(filter 1,$(DEV_LOG)),\
	RUST_LOG=$(if $(RUST_LOG),$(RUST_LOG),$(DEV_RUST_LOG)) \
	RUST_BACKTRACE=$(if $(RUST_BACKTRACE),$(RUST_BACKTRACE),$(DEV_BACKTRACE)) \
	CTX_DAEMON_LOG_STDOUT=$(if $(CTX_DAEMON_LOG_STDOUT),$(CTX_DAEMON_LOG_STDOUT),$(DEV_LOG_STDOUT)) \
	CTX_DAEMON_LOG_STDOUT_FILTER=$(if $(CTX_DAEMON_LOG_STDOUT_FILTER),$(CTX_DAEMON_LOG_STDOUT_FILTER),$(DEV_LOG_STDOUT_FILTER)) \
	CTX_DAEMON_LOG_MAX_BYTES=$(if $(CTX_DAEMON_LOG_MAX_BYTES),$(CTX_DAEMON_LOG_MAX_BYTES),$(DEV_LOG_MAX_BYTES)) \
)
DEV_DEBUG_ENV := $(if $(filter 1,$(DEV_DEBUG)),\
	CTX_ACP_HEAP_PROFILE=$(if $(CTX_ACP_HEAP_PROFILE),$(CTX_ACP_HEAP_PROFILE),$(DEV_ACP_HEAP_PROFILE)) \
	CTX_ACP_HEAP_PROFILE_DIR=$(if $(CTX_ACP_HEAP_PROFILE_DIR),$(CTX_ACP_HEAP_PROFILE_DIR),$(CTX_DAEMON_DATA_DIR)/logs/providers/heap) \
	CTX_ACP_HEAP_PROFILE_LG_INTERVAL=$(if $(CTX_ACP_HEAP_PROFILE_LG_INTERVAL),$(CTX_ACP_HEAP_PROFILE_LG_INTERVAL),$(DEV_ACP_HEAP_PROFILE_LG_INTERVAL)) \
	CTX_ACP_HEAP_PROFILE_LG_SAMPLE=$(if $(CTX_ACP_HEAP_PROFILE_LG_SAMPLE),$(CTX_ACP_HEAP_PROFILE_LG_SAMPLE),$(DEV_ACP_HEAP_PROFILE_LG_SAMPLE)) \
	CTX_RESOURCE_TELEMETRY_INTERVAL_MS=$(if $(CTX_RESOURCE_TELEMETRY_INTERVAL_MS),$(CTX_RESOURCE_TELEMETRY_INTERVAL_MS),$(DEV_RESOURCE_TELEMETRY_INTERVAL_MS)) \
	CTX_RESOURCE_TELEMETRY_LOCAL_MAX_BYTES=$(if $(CTX_RESOURCE_TELEMETRY_LOCAL_MAX_BYTES),$(CTX_RESOURCE_TELEMETRY_LOCAL_MAX_BYTES),$(DEV_RESOURCE_TELEMETRY_LOCAL_MAX_BYTES)) \
)
DEV_LOG_SETENV := $(if $(filter 1,$(DEV_LOG)),\
	--setenv RUST_LOG=$(if $(RUST_LOG),$(RUST_LOG),$(DEV_RUST_LOG)) \
	--setenv RUST_BACKTRACE=$(if $(RUST_BACKTRACE),$(RUST_BACKTRACE),$(DEV_BACKTRACE)) \
	--setenv CTX_DAEMON_LOG_STDOUT=$(if $(CTX_DAEMON_LOG_STDOUT),$(CTX_DAEMON_LOG_STDOUT),$(DEV_LOG_STDOUT)) \
	--setenv CTX_DAEMON_LOG_STDOUT_FILTER=$(if $(CTX_DAEMON_LOG_STDOUT_FILTER),$(CTX_DAEMON_LOG_STDOUT_FILTER),$(DEV_LOG_STDOUT_FILTER)) \
	--setenv CTX_DAEMON_LOG_MAX_BYTES=$(if $(CTX_DAEMON_LOG_MAX_BYTES),$(CTX_DAEMON_LOG_MAX_BYTES),$(DEV_LOG_MAX_BYTES)) \
)
DEV_DEBUG_SETENV := $(if $(filter 1,$(DEV_DEBUG)),\
	--setenv CTX_ACP_HEAP_PROFILE=$(if $(CTX_ACP_HEAP_PROFILE),$(CTX_ACP_HEAP_PROFILE),$(DEV_ACP_HEAP_PROFILE)) \
	--setenv CTX_ACP_HEAP_PROFILE_DIR=$(if $(CTX_ACP_HEAP_PROFILE_DIR),$(CTX_ACP_HEAP_PROFILE_DIR),$(CTX_DAEMON_DATA_DIR)/logs/providers/heap) \
	--setenv CTX_ACP_HEAP_PROFILE_LG_INTERVAL=$(if $(CTX_ACP_HEAP_PROFILE_LG_INTERVAL),$(CTX_ACP_HEAP_PROFILE_LG_INTERVAL),$(DEV_ACP_HEAP_PROFILE_LG_INTERVAL)) \
	--setenv CTX_ACP_HEAP_PROFILE_LG_SAMPLE=$(if $(CTX_ACP_HEAP_PROFILE_LG_SAMPLE),$(CTX_ACP_HEAP_PROFILE_LG_SAMPLE),$(DEV_ACP_HEAP_PROFILE_LG_SAMPLE)) \
	--setenv CTX_RESOURCE_TELEMETRY_INTERVAL_MS=$(if $(CTX_RESOURCE_TELEMETRY_INTERVAL_MS),$(CTX_RESOURCE_TELEMETRY_INTERVAL_MS),$(DEV_RESOURCE_TELEMETRY_INTERVAL_MS)) \
	--setenv CTX_RESOURCE_TELEMETRY_LOCAL_MAX_BYTES=$(if $(CTX_RESOURCE_TELEMETRY_LOCAL_MAX_BYTES),$(CTX_RESOURCE_TELEMETRY_LOCAL_MAX_BYTES),$(DEV_RESOURCE_TELEMETRY_LOCAL_MAX_BYTES)) \
)
DEV_CRP_ENV := CTX_ENABLE_CODEX_CRP=$(if $(CTX_ENABLE_CODEX_CRP),$(CTX_ENABLE_CODEX_CRP),$(DEV_ENABLE_CODEX_CRP))
DEV_CRP_SETENV := --setenv CTX_ENABLE_CODEX_CRP=$(if $(CTX_ENABLE_CODEX_CRP),$(CTX_ENABLE_CODEX_CRP),$(DEV_ENABLE_CODEX_CRP))
DEV_MISC_ENV := CTX_DEV_MODE=$(if $(CTX_DEV_MODE),$(CTX_DEV_MODE),$(DEV_MODE))
DEV_MISC_SETENV := --setenv CTX_DEV_MODE=$(if $(CTX_DEV_MODE),$(CTX_DEV_MODE),$(DEV_MODE))
DEV_MALLOC_CONF_ENV := $(if $(CTX_DAEMON_MALLOC_CONF),MALLOC_CONF=$(CTX_DAEMON_MALLOC_CONF))
DEV_MALLOC_CONF_SETENV := $(if $(CTX_DAEMON_MALLOC_CONF),--setenv MALLOC_CONF=$(CTX_DAEMON_MALLOC_CONF))
DEV_DAEMON_FEATURES := $(if $(CTX_DAEMON_FEATURES),--features $(CTX_DAEMON_FEATURES))
LOCAL_CODEX_CRP_ENABLED = $(if $(filter 1 true TRUE yes YES on ON,$(CTX_INSTALL_LOCAL_CODEX_CRP)),1,0)
DEV_CRP_TURBO_TASKS = crp:claude $(if $(filter 1,$(LOCAL_CODEX_CRP_ENABLED)),crp:codex,)

ifeq ($(SUPABASE_PROD),1)
  SUPABASE_LOCAL := 0
  SUPABASE_FUNCTIONS := 0
endif

test:
	@$(CTX_CACHE_EXPORT_CMD); \
	$(CTX_CACHE_RUN) cargo test -q
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
	@CTX_CRP_PROFILE="$(CTX_CRP_PROFILE)" pnpm turbo run $(DEV_CRP_TURBO_TASKS) --filter=ctx-monorepo --cache-dir=$${TURBO_CACHE_DIR:-$$HOME/.cache/turbo/ctx-monorepo} --output-logs=new-only --log-order=grouped --concurrency=1 --ui=stream --cache=local:r,remote:r
	@CTX_DATA_DIR=$(CTX_DAEMON_DATA_DIR) CTX_CRP_PROFILE="$(CTX_CRP_PROFILE)" CTX_INSTALL_LOCAL_CODEX_CRP="$(CTX_INSTALL_LOCAL_CODEX_CRP)" ../scripts/dev_install_crp_harnesses.sh
	@$(CTX_CACHE_EXPORT_CMD); \
	$(CTX_CACHE_RUN) cargo build -p ctx-mcp
ifeq ($(shell uname -s),Linux)
	@if command -v systemd-run >/dev/null 2>&1 && systemctl --user show-environment >/dev/null 2>&1; then \
		$(CTX_CACHE_EXPORT_CMD); \
		bundle_dir="$$(../scripts/ensure_bundled_harnesses.sh)"; \
		ctx_mcp_command="$${CTX_MCP_COMMAND:-$${CARGO_TARGET_DIR}/debug/ctx-mcp}"; \
		if [ "$(CTX_DAEMON_STOP)" = "1" ]; then \
			systemctl --user stop $(CTX_DAEMON_UNIT).scope >/dev/null 2>&1 || true; \
			systemctl --user reset-failed $(CTX_DAEMON_UNIT).scope >/dev/null 2>&1 || true; \
		fi; \
		systemd-run --user --scope --unit $(CTX_DAEMON_UNIT) --same-dir --collect \
			$(DEV_LOG_SETENV) \
			$(DEV_DEBUG_SETENV) \
			$(DEV_CRP_SETENV) \
			$(DEV_MISC_SETENV) \
			$(DEV_MALLOC_CONF_SETENV) \
			--setenv CTX_SESSION_ID="$(CTX_MAKE_SESSION_ID)" \
			--setenv CTX_CACHE_SCOPE_KEY="$(CTX_MAKE_SESSION_ID)" \
			--setenv CTX_MCP_COMMAND="$$ctx_mcp_command" \
			--setenv CTX_BUNDLE_DIR="$$bundle_dir" \
			node scripts/run_with_ctx_cache_env.cjs --mode workspace --cwd "$(CURDIR)" -- cargo run -p ctx-http $(DEV_DAEMON_FEATURES) --bin ctx -- serve --bind $(CTX_DAEMON_BIND) $(if $(CTX_DATA_DIR),--data-dir $(CTX_DATA_DIR),); \
	else \
		$(CTX_CACHE_EXPORT_CMD); \
		bundle_dir="$$(../scripts/ensure_bundled_harnesses.sh)"; \
		ctx_mcp_command="$${CTX_MCP_COMMAND:-$${CARGO_TARGET_DIR}/debug/ctx-mcp}"; \
		$(DEV_LOG_ENV) $(DEV_DEBUG_ENV) $(DEV_CRP_ENV) $(DEV_MISC_ENV) $(DEV_MALLOC_CONF_ENV) CTX_SESSION_ID="$(CTX_MAKE_SESSION_ID)" CTX_CACHE_SCOPE_KEY="$(CTX_MAKE_SESSION_ID)" CTX_BUNDLE_DIR="$$bundle_dir" CTX_MCP_COMMAND="$$ctx_mcp_command" node scripts/run_with_ctx_cache_env.cjs --mode workspace --cwd "$(CURDIR)" -- cargo run -p ctx-http $(DEV_DAEMON_FEATURES) --bin ctx -- serve --bind $(CTX_DAEMON_BIND) $(if $(CTX_DATA_DIR),--data-dir $(CTX_DATA_DIR),); \
	fi
else
	@$(CTX_CACHE_EXPORT_CMD); \
	bundle_dir="$$(../scripts/ensure_bundled_harnesses.sh)"; \
	ctx_mcp_command="$${CTX_MCP_COMMAND:-$${CARGO_TARGET_DIR}/debug/ctx-mcp}"; \
	$(DEV_LOG_ENV) $(DEV_DEBUG_ENV) $(DEV_CRP_ENV) $(DEV_MISC_ENV) $(DEV_MALLOC_CONF_ENV) CTX_SESSION_ID="$(CTX_MAKE_SESSION_ID)" CTX_CACHE_SCOPE_KEY="$(CTX_MAKE_SESSION_ID)" CTX_BUNDLE_DIR="$$bundle_dir" CTX_MCP_COMMAND="$$ctx_mcp_command" node scripts/run_with_ctx_cache_env.cjs --mode workspace --cwd "$(CURDIR)" -- cargo run -p ctx-http $(DEV_DAEMON_FEATURES) --bin ctx -- serve --bind $(CTX_DAEMON_BIND) $(if $(CTX_DATA_DIR),--data-dir $(CTX_DATA_DIR),)
endif

dev-codex-crp-local: CTX_INSTALL_LOCAL_CODEX_CRP=1
dev-codex-crp-local: dev

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
