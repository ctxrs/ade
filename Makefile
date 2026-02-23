.DEFAULT_GOAL := help

.PHONY: help dev verify-quick mintlify-pull mintlify-push desktop-profile-build desktop-profile-launch desktop-profile-dev

PNPM ?= pnpm
PROFILE ?= dev
DESKTOP_SYNC_BUNDLES ?= 1
DESKTOP_DEV_WEB_HOST ?= 127.0.0.1
DESKTOP_DEV_WEB_PORT ?= 5173
SAFE_PROFILE := $(shell printf '%s' "$(PROFILE)" | tr '[:upper:]' '[:lower:]' | tr -cs 'a-z0-9' '-' | sed 's/^-*//;s/-*$$//')
APP_LABEL := ctx [$(SAFE_PROFILE)]
APP_ID := rs.ctx.desktop.dev.$(SAFE_PROFILE)
PROFILE_ROOT := $(HOME)/.ctx/profiles/$(SAFE_PROFILE)
DAEMON_DIR := $(PROFILE_ROOT)/daemon
APP_DEST_DIR := $(HOME)/Applications/ctx-profiles
APP_DEST_MAC := $(APP_DEST_DIR)/ctx-$(SAFE_PROFILE).app
LINUX_BIN := /tmp/ctx-desktop-profile-$(SAFE_PROFILE)
WINDOWS_EXE := $(USERPROFILE)\AppData\Local\ctx-profiles\ctx-$(SAFE_PROFILE).exe

help:
	@echo "ctx-monorepo Make targets"
	@echo
	@echo "  dev                     Run daemon dev server (proxy to core/Makefile)"
	@echo "  verify-quick            Run workspace quick verification (pnpm -C core verify:quick)"
	@echo "  mintlify-pull           Pull Mintlify editor changes into mintlify-docs/"
	@echo "  mintlify-push           Push mintlify-docs/ to the mirror repo"
	@echo "  desktop-profile-build   Build named desktop profile (PROFILE=<name>)"
	@echo "  desktop-profile-launch  Build + launch named desktop profile (PROFILE=<name>)"
	@echo "  desktop-profile-dev     Run profile-scoped tauri+web hot-reload loop (PROFILE=<name>)"

dev:
	$(MAKE) -C core dev

verify-quick:
	$(PNPM) -C core verify:quick

mintlify-pull:
	./scripts/mintlify/pull-subtree.sh

mintlify-push:
	./scripts/mintlify/push-subtree.sh

desktop-profile-build:
	@set -euo pipefail; \
	if [ -z "$(SAFE_PROFILE)" ]; then \
		echo "PROFILE must contain at least one alphanumeric character."; \
		exit 1; \
	fi; \
	mkdir -p "$(PROFILE_ROOT)" "$(DAEMON_DIR)"; \
	CTX_DESKTOP_SYNC_BUNDLES="$(DESKTOP_SYNC_BUNDLES)" $(PNPM) -C core desktop:prep; \
	CONFIG_JSON="$$(jq -nc --arg pn "$(APP_LABEL)" --arg id "$(APP_ID)" '{productName:$$pn,identifier:$$id}')"; \
	$(PNPM) -C core/apps/desktop exec tauri build --debug --config "$$CONFIG_JSON"; \
	UNAME="$$(uname -s 2>/dev/null || echo unknown)"; \
	case "$$UNAME" in \
		Darwin) \
			mkdir -p "$(APP_DEST_DIR)"; \
			SRC_APP="$$(ls -dt core/apps/desktop/src-tauri/target/debug/bundle/macos/*.app | head -n 1)"; \
			rm -rf "$(APP_DEST_MAC)"; \
			cp -R "$$SRC_APP" "$(APP_DEST_MAC)"; \
			echo "Built macOS profile app: $(APP_DEST_MAC)"; \
			;; \
		Linux) \
			cp -f core/apps/desktop/src-tauri/target/debug/ctx "$(LINUX_BIN)"; \
			chmod +x "$(LINUX_BIN)"; \
			echo "Built Linux profile binary: $(LINUX_BIN)"; \
			;; \
		*) \
			if [ "$${OS:-}" = "Windows_NT" ]; then \
				mkdir -p "$$(dirname "$(WINDOWS_EXE)")"; \
				cp -f core/apps/desktop/src-tauri/target/debug/ctx.exe "$(WINDOWS_EXE)"; \
				echo "Built Windows profile binary: $(WINDOWS_EXE)"; \
			else \
				echo "Built profile artifacts, but OS launch packaging is not configured for '$$UNAME'."; \
			fi; \
			;; \
	esac

desktop-profile-launch: desktop-profile-build
	@set -euo pipefail; \
	UNAME="$$(uname -s 2>/dev/null || echo unknown)"; \
	case "$$UNAME" in \
		Darwin) \
			open -n --env CTX_DESKTOP_DAEMON_DATA_DIR="$(DAEMON_DIR)" "$(APP_DEST_MAC)"; \
			echo "Launched macOS profile '$(SAFE_PROFILE)' with data dir: $(DAEMON_DIR)"; \
			;; \
		Linux) \
			CTX_DESKTOP_DAEMON_DATA_DIR="$(DAEMON_DIR)" nohup "$(LINUX_BIN)" >/tmp/ctx-desktop-profile-$(SAFE_PROFILE).log 2>&1 & \
			echo "Launched Linux profile '$(SAFE_PROFILE)' with data dir: $(DAEMON_DIR)"; \
			;; \
		*) \
			if [ "$${OS:-}" = "Windows_NT" ]; then \
				CTX_DESKTOP_DAEMON_DATA_DIR="$(DAEMON_DIR)" powershell -NoProfile -Command "Start-Process -FilePath '$(WINDOWS_EXE)'"; \
				echo "Launched Windows profile '$(SAFE_PROFILE)' with data dir: $(DAEMON_DIR)"; \
			else \
				echo "Launch not configured for this OS."; \
				exit 1; \
			fi; \
			;; \
	esac

desktop-profile-dev:
	@set -euo pipefail; \
	if [ -z "$(SAFE_PROFILE)" ]; then \
		echo "PROFILE must contain at least one alphanumeric character."; \
		exit 1; \
	fi; \
	mkdir -p "$(PROFILE_ROOT)" "$(DAEMON_DIR)"; \
	if pgrep -f -- "--data-dir $(DAEMON_DIR)" >/dev/null 2>&1; then \
		echo "A daemon is already running for profile '$(SAFE_PROFILE)' ($(DAEMON_DIR))."; \
		echo "Close that app/process first, then rerun desktop-profile-dev."; \
		exit 1; \
	fi; \
	CTX_DESKTOP_SYNC_BUNDLES="$(DESKTOP_SYNC_BUNDLES)" $(PNPM) -C core desktop:prep:dev; \
	WEB_HOST="$(DESKTOP_DEV_WEB_HOST)"; \
	WEB_PORT="$(DESKTOP_DEV_WEB_PORT)"; \
	DEV_URL="http://$$WEB_HOST:$$WEB_PORT"; \
	CONFIG_JSON="$$(jq -nc --arg pn "$(APP_LABEL)" --arg id "$(APP_ID)" --arg dev "$$DEV_URL" '{productName:$$pn,identifier:$$id,build:{devUrl:$$dev}}')"; \
	CTX_DEV_HTTP=1 $(PNPM) -C core/apps/web dev --host "$$WEB_HOST" --port "$$WEB_PORT" >/tmp/ctx-web-$(SAFE_PROFILE).log 2>&1 & \
	WEB_PID=$$!; \
	cleanup() { \
		kill "$$WEB_PID" >/dev/null 2>&1 || true; \
		wait "$$WEB_PID" >/dev/null 2>&1 || true; \
	}; \
	trap cleanup EXIT INT TERM; \
	if command -v curl >/dev/null 2>&1; then \
		for _ in $$(seq 1 60); do \
			if curl -fsS "$$DEV_URL" >/dev/null 2>&1; then \
				break; \
			fi; \
			sleep 1; \
		done; \
	else \
		sleep 2; \
	fi; \
	echo "Starting tauri dev for profile '$(SAFE_PROFILE)' (web: $$DEV_URL, data dir: $(DAEMON_DIR))"; \
	CTX_DESKTOP_DAEMON_DATA_DIR="$(DAEMON_DIR)" $(PNPM) -C core/apps/desktop exec tauri dev --config "$$CONFIG_JSON"
