.PHONY: test dev web

test:
	cargo test -q
	pnpm -C apps/web test

dev:
ifeq ($(shell uname -s),Linux)
	@if command -v systemd-run >/dev/null 2>&1 && systemctl --user show-environment >/dev/null 2>&1; then \
		systemd-run --user --scope --unit context-daemon --same-dir \
			cargo run -p context-http --bin context -- serve; \
	else \
		cargo run -p context-http --bin context -- serve; \
	fi
else
	cargo run -p context-http --bin context -- serve
endif

web:
	pnpm -C apps/web dev
