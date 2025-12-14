.PHONY: test dev web

test:
	cargo test -q
	pnpm -C apps/web test

dev:
	cargo run -p context-http --bin context -- serve

web:
	pnpm -C apps/web dev
