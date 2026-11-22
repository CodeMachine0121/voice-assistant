# Voice Assistant — build, install and development tasks.
# Run `make help` to list targets.

# Per-machine settings (git-ignored), e.g. `CODESIGN_IDENTITY := My Cert`.
-include .make.local

APP_NAME    := Voice Assistant
BINARY      := voice-assistant
PROFILE     ?= release
APP         := target/$(PROFILE)/$(APP_NAME).app
INSTALL_DIR ?= /Applications
# Optional: name of a code-signing certificate in your keychain, so macOS keeps
# permissions across rebuilds (ad-hoc signing is used when empty). Set it on the
# command line, in the environment, or once in .make.local.
CODESIGN_IDENTITY ?=
export CODESIGN_IDENTITY
# Same target for every build so cargo never rebuilds whisper.cpp when switching
# between `make run` and `make build`.
export MACOSX_DEPLOYMENT_TARGET := 13.0

.DEFAULT_GOAL := help
.PHONY: help deps build debug open install uninstall quit run demo test lint fmt check clean

help: ## Show this help
	@awk 'BEGIN {FS = ":.*## "} /^[a-zA-Z_-]+:.*## / {printf "  \033[36m%-10s\033[0m %s\n", $$1, $$2}' $(MAKEFILE_LIST)

deps: ## Check build prerequisites (Xcode CLT, CMake, Rust)
	@xcode-select -p >/dev/null 2>&1 || { echo "Missing Xcode Command Line Tools: run 'xcode-select --install'"; exit 1; }
	@command -v cmake >/dev/null || { echo "Missing CMake: run 'brew install cmake'"; exit 1; }
	@command -v cargo >/dev/null || { echo "Missing Rust: see https://rustup.rs"; exit 1; }
	@echo "All prerequisites found."

build: deps ## Build the app bundle (target/<profile>/Voice Assistant.app; PROFILE=release or debug)
	@scripts/bundle.sh $(PROFILE)

debug: ## Build a debug app bundle (target/debug/Voice Assistant.app)
	@$(MAKE) --no-print-directory build PROFILE=debug

quit: ## Quit the app if it is running
	@pkill -x $(BINARY) 2>/dev/null && sleep 1 || true

open: build quit ## Build and launch the app (restarting a running one)
	@open "$(APP)"

install: build quit ## Build and copy the app to /Applications (override with INSTALL_DIR=...)
	@rm -rf "$(INSTALL_DIR)/$(APP_NAME).app"
	@cp -R "$(APP)" "$(INSTALL_DIR)/"
	@echo "Installed $(INSTALL_DIR)/$(APP_NAME).app"

uninstall: quit ## Remove the app from /Applications (settings and models are kept)
	@rm -rf "$(INSTALL_DIR)/$(APP_NAME).app"
	@echo "Removed $(INSTALL_DIR)/$(APP_NAME).app"

run: ## Run the app from source (logs to the terminal)
	@RUST_LOG=$${RUST_LOG:-info} cargo run -p voice-assistant

demo: ## Run with simulated microphone, speech and assistant (no permissions needed)
	@VOICE_ASSISTANT_DEMO=1 RUST_LOG=$${RUST_LOG:-info} cargo run -p voice-assistant

test: ## Run all tests
	@cargo test --workspace

lint: ## Run clippy with warnings as errors
	@cargo clippy --workspace --all-targets -- -D warnings

fmt: ## Format the code
	@cargo fmt --all

check: ## Format check, lint and test (what a PR must pass)
	@cargo fmt --all -- --check
	@$(MAKE) --no-print-directory lint
	@$(MAKE) --no-print-directory test

clean: ## Remove build output
	@cargo clean
