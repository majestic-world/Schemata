.DEFAULT_GOAL := build

.PHONY: build linux mac

# Build for the current host OS and architecture.
build:
	node scripts/build.mjs native

# Cross-platform packaging runs exclusively in Linux containers.
linux:
	node scripts/build.mjs linux

mac:
	node scripts/build.mjs mac
