.DEFAULT_GOAL := help

.PHONY: help dev verify-quick native-dev native-fmt native-check native-build native-run native-clippy native-clean

CARGO ?= cargo
PNPM ?= pnpm

NATIVE_MANIFEST := core/apps/native/Cargo.toml
NATIVE_RUSTFLAGS ?= -Dwarnings
NATIVE_DEV_STRICT ?= 1
NATIVE_FEATURES ?=
NATIVE_ARGS ?=

help:
	@echo "ctx-monorepo Make targets"
	@echo
	@echo "  dev              Run daemon dev server (proxy to core/Makefile)"
	@echo "  verify-quick      Run workspace quick verification (pnpm -C core verify:quick)"
	@echo
	@echo "  native-dev        Fast launch native app (does not start daemon)"
	@echo "  native-fmt        cargo fmt for core/apps/native"
	@echo "  native-check      cargo check for core/apps/native (RUSTFLAGS=-Dwarnings by default)"
	@echo "  native-build      cargo build for core/apps/native (RUSTFLAGS=-Dwarnings by default)"
	@echo "  native-run        cargo run for core/apps/native (does not start daemon)"
	@echo "  native-clippy     cargo clippy for core/apps/native (-D warnings)"
	@echo "  native-clean      cargo clean for core/apps/native"
	@echo
	@echo "Useful vars:"
	@echo "  CTX_DAEMON_URL=... CTX_DATA_DIR=...   (for native-run)"
	@echo "  NATIVE_FEATURES=automation            (opt-in feature flags)"
	@echo "  NATIVE_DEV_STRICT=0                   (faster dev, but may allow warnings)"
	@echo "  NATIVE_RUSTFLAGS='-Dwarnings'         (override warnings policy)"
	@echo "  NATIVE_ARGS='--help'                  (args passed after --)"

dev:
	$(MAKE) -C core dev

verify-quick:
	$(PNPM) -C core verify:quick

native-dev:
	@env \
		$(if $(CTX_DAEMON_URL),CTX_DAEMON_URL="$(CTX_DAEMON_URL)",) \
		$(if $(CTX_DATA_DIR),CTX_DATA_DIR="$(CTX_DATA_DIR)",) \
		RUSTFLAGS="$(if $(filter 1,$(NATIVE_DEV_STRICT)),$(NATIVE_RUSTFLAGS),)" \
		$(CARGO) run --manifest-path $(NATIVE_MANIFEST) $(if $(NATIVE_FEATURES),--features "$(NATIVE_FEATURES)",) -- $(NATIVE_ARGS)

native-fmt:
	$(CARGO) fmt --manifest-path $(NATIVE_MANIFEST)

native-check:
	RUSTFLAGS="$(NATIVE_RUSTFLAGS)" $(CARGO) check --manifest-path $(NATIVE_MANIFEST) $(if $(NATIVE_FEATURES),--features "$(NATIVE_FEATURES)",)

native-build:
	RUSTFLAGS="$(NATIVE_RUSTFLAGS)" $(CARGO) build --manifest-path $(NATIVE_MANIFEST) $(if $(NATIVE_FEATURES),--features "$(NATIVE_FEATURES)",)

native-run:
	@env \
		$(if $(CTX_DAEMON_URL),CTX_DAEMON_URL="$(CTX_DAEMON_URL)",) \
		$(if $(CTX_DATA_DIR),CTX_DATA_DIR="$(CTX_DATA_DIR)",) \
		RUSTFLAGS="$(NATIVE_RUSTFLAGS)" \
		$(CARGO) run --manifest-path $(NATIVE_MANIFEST) $(if $(NATIVE_FEATURES),--features "$(NATIVE_FEATURES)",) -- $(NATIVE_ARGS)

native-clippy:
	$(CARGO) clippy --manifest-path $(NATIVE_MANIFEST) $(if $(NATIVE_FEATURES),--features "$(NATIVE_FEATURES)",) -- -D warnings

native-clean:
	$(CARGO) clean --manifest-path $(NATIVE_MANIFEST)
