# Thin wrappers over npm and cargo, so there is one place to look for "how do
# I run this". Nothing here is load-bearing: every target is a command you can
# also type by hand, and the underlying tools stay the source of truth.

# --manifest-path belongs to the subcommand, not to cargo itself.
CARGO := cargo
MANIFEST := --manifest-path src-tauri/Cargo.toml

.DEFAULT_GOAL := help
.PHONY: help install dev build test test-frontend test-backend check fmt lint \
        live-login local-game tiles clean

help: ## Show this help
	@grep -hE '^[a-z-]+:.*?## ' $(MAKEFILE_LIST) \
		| awk -F':.*?## ' '{printf "  \033[1m%-14s\033[0m %s\n", $$1, $$2}'
	@echo
	@echo "  First time: make install, then make dev."

## --- running ---------------------------------------------------------------

# Stamped against the lockfile so a dependency change reinstalls, and an
# unchanged tree does not pay for npm on every build.
node_modules: package-lock.json package.json
	npm install
	@touch node_modules

install: node_modules ## Install the frontend dependencies

dev: node_modules ## Run the app with hot reload
	npm run tauri dev

build: node_modules ## Build the packaged app (see src-tauri/target/release/bundle)
	npm run tauri build

## --- checking --------------------------------------------------------------

test: test-frontend test-backend ## Run every test

test-frontend: node_modules ## Frontend tests (vitest)
	npm test

test-backend: ## Backend tests (cargo)
	$(CARGO) test $(MANIFEST)

check: node_modules ## Typecheck the frontend and the Rust build
	npx tsc --noEmit
	$(CARGO) check $(MANIFEST) --all-targets

fmt: ## Format the Rust sources
	$(CARGO) fmt $(MANIFEST)

lint: ## Clippy, warnings treated as errors
	$(CARGO) clippy $(MANIFEST) --all-targets -- -D warnings

## --- things that touch the outside world ------------------------------------

# Both of these are #[ignore]d in the normal run: one needs the network and a
# real game account, the other needs a NetHack installed on this machine.

live-login: ## Log in to a real server end to end (needs NHTILES_TEST_USER/PASS)
	@test -n "$$NHTILES_TEST_USER" || { \
		echo "set NHTILES_TEST_USER and NHTILES_TEST_PASS first"; exit 1; }
	$(CARGO) test $(MANIFEST) --test live_login -- --ignored --nocapture

local-game: ## Start the locally installed NetHack in a pty and check it draws
	$(CARGO) test $(MANIFEST) --lib the_installed_nethack_starts_and_draws -- --ignored --nocapture

tiles: ## Rebuild a tile sheet, e.g. make tiles ARGS="--version v36 --out ..."
	$(CARGO) run $(MANIFEST) --bin tiles2png -- $(ARGS)

clean: ## Remove build output
	rm -rf dist src-tauri/target
