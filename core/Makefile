.PHONY: test dev web

test:
\tcargo test -q
\tpnpm -C apps/web test

dev:
\tcargo run -p context-http -- serve

web:
\tpnpm -C apps/web dev

