.DEFAULT_GOAL := help

.PHONY: help dev verify-quick native-dev native-dev-watch native-fmt native-check native-build native-run native-clippy native-clean native-remote-setup native-remote-dev-watch

CARGO ?= cargo
PNPM ?= pnpm

NATIVE_MANIFEST := core/apps/native/Cargo.toml
NATIVE_RUSTFLAGS ?= -Dwarnings
NATIVE_DEV_STRICT ?= 1
NATIVE_FEATURES ?=
NATIVE_ARGS ?=
NATIVE_WATCH_PATHS ?= core/apps/native core/crates
NATIVE_WATCH_EXTS ?= rs,toml
NATIVE_CARGO_TARGET_DIR := $(if $(CARGO_TARGET_DIR),$(CARGO_TARGET_DIR),$(HOME)/.cache/cargo/ctx-monorepo/$(shell basename "$(shell git rev-parse --git-dir)"))
NATIVE_DEV_RUSTFLAGS := $(if $(filter 1,$(NATIVE_DEV_STRICT)),$(NATIVE_RUSTFLAGS),)
NATIVE_BIN := $(NATIVE_CARGO_TARGET_DIR)/debug/ctx

NATIVE_REMOTE_SSH ?=
NATIVE_REMOTE_LOCAL_PORT ?= 4399
NATIVE_REMOTE_DAEMON_HOST ?= 127.0.0.1
NATIVE_REMOTE_DAEMON_PORT ?= 4399
NATIVE_REMOTE_AUTH_PATH ?= ~/.ctx/daemon_auth.json
NATIVE_REMOTE_CTX_DATA_DIR ?= /tmp/ctx-remote
NATIVE_REMOTE_TMUX_SESSION ?= ctx-native-remote-daemon-tunnel

help:
	@echo "ctx-monorepo Make targets"
	@echo
	@echo "  dev              Run daemon dev server (proxy to core/Makefile)"
	@echo "  verify-quick      Run workspace quick verification (pnpm -C core verify:quick)"
	@echo
	@echo "  native-dev        Fast launch native app (does not start daemon)"
	@echo "  native-dev-watch  Rebuild+restart native app on changes"
	@echo "  native-fmt        cargo fmt for core/apps/native"
	@echo "  native-check      cargo check for core/apps/native (RUSTFLAGS=-Dwarnings by default)"
	@echo "  native-build      cargo build for core/apps/native (RUSTFLAGS=-Dwarnings by default)"
	@echo "  native-run        cargo run for core/apps/native (does not start daemon)"
	@echo "  native-clippy     cargo clippy for core/apps/native (-D warnings)"
	@echo "  native-clean      cargo clean for core/apps/native"
	@echo
	@echo "  native-remote-setup     SSH-tunnel + auth sync for remote daemon"
	@echo "  native-remote-dev-watch native-dev-watch wired to remote daemon"
	@echo
	@echo "Useful vars:"
	@echo "  CTX_DAEMON_URL=... CTX_DATA_DIR=...   (for native-run)"
	@echo "  NATIVE_FEATURES=automation            (opt-in feature flags)"
	@echo "  NATIVE_DEV_STRICT=0                   (faster dev, but may allow warnings)"
	@echo "  NATIVE_RUSTFLAGS='-Dwarnings'         (override warnings policy)"
	@echo "  CARGO_TARGET_DIR=...                  (shared cargo target dir)"
	@echo "  NATIVE_ARGS='--help'                  (args passed after --)"
	@echo
	@echo "Remote vars:"
	@echo "  NATIVE_REMOTE_SSH=dev@...             (SSH target for remote daemon host)"
	@echo "  NATIVE_REMOTE_LOCAL_PORT=4399         (local forwarded port)"
	@echo "  NATIVE_REMOTE_DAEMON_HOST=127.0.0.1   (remote host to forward to)"
	@echo "  NATIVE_REMOTE_DAEMON_PORT=4399        (remote daemon port)"
	@echo "  NATIVE_REMOTE_AUTH_PATH=~/.ctx/daemon_auth.json"
	@echo "  NATIVE_REMOTE_CTX_DATA_DIR=/tmp/ctx-remote (default CTX_DATA_DIR if unset)"
	@echo "  NATIVE_REMOTE_TMUX_SESSION=...        (tmux session for tunnel, if tmux exists)"

dev:
	$(MAKE) -C core dev

verify-quick:
	$(PNPM) -C core verify:quick

native-dev:
	@env \
		$(if $(CTX_DAEMON_URL),CTX_DAEMON_URL="$(CTX_DAEMON_URL)",) \
		$(if $(CTX_DATA_DIR),CTX_DATA_DIR="$(CTX_DATA_DIR)",) \
		CARGO_TARGET_DIR="$(NATIVE_CARGO_TARGET_DIR)" \
		RUSTFLAGS="$(NATIVE_DEV_RUSTFLAGS)" \
		$(CARGO) run --manifest-path $(NATIVE_MANIFEST) $(if $(NATIVE_FEATURES),--features "$(NATIVE_FEATURES)",) -- $(NATIVE_ARGS)

native-build-dev:
	RUSTFLAGS="$(NATIVE_DEV_RUSTFLAGS)" CARGO_TARGET_DIR="$(NATIVE_CARGO_TARGET_DIR)" $(CARGO) build --manifest-path $(NATIVE_MANIFEST) $(if $(NATIVE_FEATURES),--features "$(NATIVE_FEATURES)",)

native-dev-watch:
	@command -v watchexec >/dev/null 2>&1 || { \
		echo "error: watchexec is required for native-dev-watch (brew install watchexec)"; \
		exit 1; \
	}
	@bash -lc 'set -euo pipefail; \
		pidfile="$$(mktemp -t ctx-native-dev-watch.XXXXXX)"; \
		echo "native-dev-watch: pidfile=$$pidfile"; \
		cleanup() { \
			if [ -f "$$pidfile" ]; then \
				pid="$$(cat "$$pidfile" 2>/dev/null || true)"; \
				if [ -n "$$pid" ]; then kill "$$pid" 2>/dev/null || true; fi; \
				rm -f "$$pidfile"; \
			fi; \
		}; \
		trap cleanup EXIT INT TERM; \
		$(MAKE) native-build-dev; \
		env \
			$(if $(CTX_DAEMON_URL),CTX_DAEMON_URL="$(CTX_DAEMON_URL)",) \
			$(if $(CTX_DATA_DIR),CTX_DATA_DIR="$(CTX_DATA_DIR)",) \
			"$(NATIVE_BIN)" $(NATIVE_ARGS) & echo $$! > "$$pidfile"; \
		watchexec \
			--postpone \
			--on-busy-update=queue \
			--debounce 200ms \
			--clear \
			$(foreach p,$(NATIVE_WATCH_PATHS),--watch $(p)) \
			--exts $(NATIVE_WATCH_EXTS) \
			-- bash -lc "set -euo pipefail; $(MAKE) native-build-dev; pid=\$$(cat \"$$pidfile\" 2>/dev/null || true); if [ -n \"\$$pid\" ]; then kill \"\$$pid\" 2>/dev/null || true; fi; env $(if $(CTX_DAEMON_URL),CTX_DAEMON_URL=\"$(CTX_DAEMON_URL)\",) $(if $(CTX_DATA_DIR),CTX_DATA_DIR=\"$(CTX_DATA_DIR)\",) \"$(NATIVE_BIN)\" $(NATIVE_ARGS) & echo \$$! > \"$$pidfile\""; \
		wait'

native-fmt:
	$(CARGO) fmt --manifest-path $(NATIVE_MANIFEST)

native-check:
	RUSTFLAGS="$(NATIVE_RUSTFLAGS)" CARGO_TARGET_DIR="$(NATIVE_CARGO_TARGET_DIR)" $(CARGO) check --manifest-path $(NATIVE_MANIFEST) $(if $(NATIVE_FEATURES),--features "$(NATIVE_FEATURES)",)

native-build:
	RUSTFLAGS="$(NATIVE_RUSTFLAGS)" CARGO_TARGET_DIR="$(NATIVE_CARGO_TARGET_DIR)" $(CARGO) build --manifest-path $(NATIVE_MANIFEST) $(if $(NATIVE_FEATURES),--features "$(NATIVE_FEATURES)",)

native-run:
	@env \
		$(if $(CTX_DAEMON_URL),CTX_DAEMON_URL="$(CTX_DAEMON_URL)",) \
		$(if $(CTX_DATA_DIR),CTX_DATA_DIR="$(CTX_DATA_DIR)",) \
		CARGO_TARGET_DIR="$(NATIVE_CARGO_TARGET_DIR)" \
		RUSTFLAGS="$(NATIVE_RUSTFLAGS)" \
		$(CARGO) run --manifest-path $(NATIVE_MANIFEST) $(if $(NATIVE_FEATURES),--features "$(NATIVE_FEATURES)",) -- $(NATIVE_ARGS)

native-clippy:
	CARGO_TARGET_DIR="$(NATIVE_CARGO_TARGET_DIR)" $(CARGO) clippy --manifest-path $(NATIVE_MANIFEST) $(if $(NATIVE_FEATURES),--features "$(NATIVE_FEATURES)",) -- -D warnings

native-clean:
	$(CARGO) clean --manifest-path $(NATIVE_MANIFEST)

native-remote-setup:
	@bash -lc 'set -euo pipefail; \
		remote="$(NATIVE_REMOTE_SSH)"; \
		[ -n "$$remote" ] || { echo "Set NATIVE_REMOTE_SSH=user@host"; exit 1; }; \
		local_port="$(NATIVE_REMOTE_LOCAL_PORT)"; \
		remote_host="$(NATIVE_REMOTE_DAEMON_HOST)"; \
		remote_port="$(NATIVE_REMOTE_DAEMON_PORT)"; \
		auth_src="$(NATIVE_REMOTE_AUTH_PATH)"; \
		data_dir="$${CTX_DATA_DIR:-"$(NATIVE_REMOTE_CTX_DATA_DIR)"}"; \
		mkdir -p "$$data_dir"; \
		chmod 700 "$$data_dir" 2>/dev/null || true; \
		scp -q "$$remote:$$auth_src" "$$data_dir/daemon_auth.json"; \
		chmod 600 "$$data_dir/daemon_auth.json"; \
		if command -v tmux >/dev/null 2>&1; then \
			session="$(NATIVE_REMOTE_TMUX_SESSION)"; \
			if ! tmux has-session -t "$$session" 2>/dev/null; then \
				tmux new-session -d -s "$$session" "ssh -N -o ExitOnForwardFailure=yes -L $$local_port:$$remote_host:$$remote_port $$remote"; \
			fi; \
		else \
			ssh -fN -o ExitOnForwardFailure=yes -L "$$local_port:$$remote_host:$$remote_port" "$$remote"; \
		fi; \
		if ! nc -z 127.0.0.1 "$$local_port" >/dev/null 2>&1; then \
			echo "error: daemon tunnel not listening on 127.0.0.1:$$local_port"; \
			echo "hint: try setting NATIVE_REMOTE_LOCAL_PORT=4440 if 4399 is in use"; \
			exit 1; \
		fi; \
		echo "ok: synced auth -> $$data_dir/daemon_auth.json"; \
		echo "ok: tunnel -> http://127.0.0.1:$$local_port"; \
		echo "next: CTX_DATA_DIR=$$data_dir CTX_DAEMON_URL=http://127.0.0.1:$$local_port make native-dev-watch"; \
	'

native-remote-dev-watch: native-remote-setup
	@bash -lc 'set -euo pipefail; \
		data_dir="$${CTX_DATA_DIR:-"$(NATIVE_REMOTE_CTX_DATA_DIR)"}"; \
		CTX_DATA_DIR="$$data_dir" CTX_DAEMON_URL="http://127.0.0.1:$(NATIVE_REMOTE_LOCAL_PORT)" $(MAKE) native-dev-watch; \
	'
