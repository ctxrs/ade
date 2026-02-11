.DEFAULT_GOAL := help

.PHONY: help dev verify-quick

PNPM ?= pnpm

help:
	@echo "ctx-monorepo Make targets"
	@echo
	@echo "  dev             Run daemon dev server (proxy to core/Makefile)"
	@echo "  verify-quick    Run workspace quick verification (pnpm -C core verify:quick)"

dev:
	$(MAKE) -C core dev

verify-quick:
	$(PNPM) -C core verify:quick
