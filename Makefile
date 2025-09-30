.PHONY: help

# absolute path to this makefile
mkfile_path := $(dir $(abspath $(lastword $(MAKEFILE_LIST))))

subxt_cli_version := 0.44.0

help: # Show help for each of the Makefile recipes
	@grep -E '^[a-zA-Z0-9 -]+:.*#'  Makefile | sort | while read -r l; do printf "\033[1;32m$$(echo $$l | cut -f 1 -d':')\033[00m:$$(echo $$l | cut -f 2- -d'#')\n"; done

#####################
### Setup Project ###
#####################

install-subxt-cli: # Install subxt-cli into the project directory
	cargo install --root $(mkfile_path) --version $(subxt_cli_version) --locked subxt-cli

# TODO: read URL from json config and/or env var instead of hardcode
download-node-metadata: # Download metadata of configured Asset Hub node. Required for subxt compilation.
	PATH="${PWD}/bin:${PATH}" subxt metadata -f bytes --url ws://localhost:9000 > metadata.scale