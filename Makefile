.PHONY: help

# absolute path to this makefile
mkfile_path := $(dir $(abspath $(lastword $(MAKEFILE_LIST))))

# Keep in sync with subxt version in Cargo.toml
subxt_cli_version := 0.44.0

help: # Show help for each of the Makefile recipes
	@grep -E '^[a-zA-Z0-9 -]+:.*#'  Makefile | sort | while read -r l; do printf "\033[1;32m$$(echo $$l | cut -f 1 -d':')\033[00m:$$(echo $$l | cut -f 2- -d'#')\n"; done

#####################
### Setup Project ###
#####################

install-subxt-cli: # Install subxt-cli into the project directory
	cargo install --root $(mkfile_path) --version $(subxt_cli_version) --locked subxt-cli

# TODO: read URL from json config and/or env var instead of hardcode
download-node-metadata: # Download metadata of configured Asset Hub node. Required for subxt compilation. By default use ws://localhost:9000 url.
	PATH="${PWD}/bin:${PATH}" subxt metadata -f bytes --url ws://localhost:9000 > metadata.scale

# TODO: read alternative value from env
download-node-metadata-ci: # Download metadata of Asset Hub node. Required for subxt compilation. By default use wss://statemint-rpc.dwellir.com url.
	PATH="${PWD}/bin:${PATH}" subxt metadata -f bytes --url wss://statemint-rpc.dwellir.com > metadata.scale

copy-configs: # Copy .example configs to actual configs
	cd configs; \
	for i in ./*.example; \
	do \
		cp "$$i" "$${i%.*}"; \
	done

copy-configs-ci: # Copy .ci configs to actual configs
	cd configs; \
	for i in ./*.example; \
	do \
		cp "$$i" "$${i%.*}"; \
	done

copy-ah-production-config: # Copy chain.json.example_asset_hub config to actual chain.json config
	cd configs; \
	cp chain.json.example_asset_hub chain.json

create-network: # Create docker network `kalatori-network` required for docker compose services
	docker network create kalatori-network || true

#####################
### Build and run ###
#####################

build-release: # Build the daemon with --release flag
	cargo build --release

start-chopsticks: # Start chopsticks for Asset Hub in docker compose with port-forwarding
	cd chopsticks; \
	docker compose up -d

stop-chopsticks: # Stop chopsticks for Asset Hub in docker compose
	cd chopsticks; \
	docker compose down

# TODO: add some health check for chopsticks to avoid errors on connection while it's not initialized
run: start-chopsticks # Ensure that chopsticks is started and run kalatori daemon locally
	cargo run

run-release: # Run kalatori daemon with --release flag without starting chopsticks
	cargo run --release

##############
### Checks ###
##############

cargo-check: # Run cargo check for all targets
	cargo check --all-targets

# Keep same as in CI
cargo-clippy: # Run cargo clippy checks
	cargo clippy --all-targets -- -D warnings -D clippy::pedantic -D clippy::correctness -D clippy::complexity -D clippy::perf

cargo-fmt: # Run cargo fmt checks
	cargo fmt --all -- --check

cargo-deny: # Run cargo deny checks
	cargo deny -L error check
