.DEFAULT_GOAL := help

.PHONY: help dev verify-quick desktop-profile-build desktop-profile-launch

PNPM ?= pnpm
PROFILE ?= dev
DESKTOP_SYNC_BUNDLES ?= 0
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
	@echo "  desktop-profile-build   Build named desktop profile (PROFILE=<name>)"
	@echo "  desktop-profile-launch  Build + launch named desktop profile (PROFILE=<name>)"

dev:
	$(MAKE) -C core dev

verify-quick:
	$(PNPM) -C core verify:quick

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
