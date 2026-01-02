.PHONY: test dev web

test:
	cargo test -q
	pnpm -C apps/web test

dev:
ifeq ($(shell uname -s),Linux)
	@if command -v systemd-run >/dev/null 2>&1 && systemctl --user show-environment >/dev/null 2>&1; then \
		systemctl --user stop ctx-daemon.scope >/dev/null 2>&1 || true; \
		systemctl --user reset-failed ctx-daemon.scope >/dev/null 2>&1 || true; \
		systemd-run --user --scope --unit ctx-daemon --same-dir --collect \
			cargo run -p ctx-http --bin ctx -- serve; \
	else \
		cargo run -p ctx-http --bin ctx -- serve; \
	fi
else
	cargo run -p ctx-http --bin ctx -- serve
endif

web:
	pnpm -C apps/web dev
