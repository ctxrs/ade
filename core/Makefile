.PHONY: test dev web

CARGO_TARGET_DIR ?= $(HOME)/.cache/cargo/ctx-monorepo
CTX_MCP_COMMAND ?= $(CARGO_TARGET_DIR)/debug/ctx-mcp
export CARGO_TARGET_DIR

test:
	cargo test -q
	pnpm -C apps/web test

dev:
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

web:
	pnpm -C apps/web dev
