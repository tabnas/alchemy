# Build and test the Rust (rs/), TypeScript (ts/) and Go (go/)
# implementations. rs/ is the reference implementation; ts/ and go/ are
# its ports, held to it by the shared fixtures in test/spec/.
#
# Local build/test resolve the unpublished tabnas siblings (transduce and
# render among them) via the repo-set go.work, the node_modules symlinks
# (admin/scripts/link.sh) and the Rust path dependencies on the sibling
# checkouts. Every runtime's tests also read transduce's fixture documents,
# ../transduce/rs/tests/fixtures.

.PHONY: all build test clean build-ts build-go build-rs test-ts test-go test-rs \
        clean-ts clean-go clean-rs embed version-rs reset bench gate

all: build test

build: build-ts build-go build-rs

test: test-ts test-go test-rs

clean: clean-ts clean-go clean-rs

# --- TypeScript (package in ts/) ---
build-ts:
	cd ts && npm run build

# npm test builds first: the script runs `npm run build` itself.
test-ts:
	cd ts && npm test

clean-ts:
	rm -rf ts/dist ts/dist-test

# --- Go (module in go/) ---
build-go:
	cd go && go build ./...

# -count=1: the shared fixtures, the grammar document and transduce's
# fixture documents live outside the module, so a change to them does not
# invalidate Go's test cache. go/clib's contract tests run here too.
test-go:
	cd go && go test -count=1 -v ./...

clean-go:
	cd go && go clean

# --- Rust (crate in rs/) ---
build-rs:
	cd rs && cargo build --all-targets

test-rs:
	cd rs && cargo test --all-targets && cargo test --doc
	cd rs && cargo clippy --all-targets --all-features -- -D warnings

clean-rs:
	cd rs && cargo clean

bench:
	cd rs && cargo bench

# The full Rust gate CI runs (.github/workflows/rust.yml): fmt, build,
# tests, doctests, clippy and the lock discipline, on the MSRV when it is
# installed.
gate:
	ci/rust/run.sh

# --- Shared sources ---
# Copy the shared sources (alchemy-grammar.jsonic, stdlib/*.alc) into
# every runtime: rs/src/grammar.rs and rs/stdlib/, ts/src/grammar.ts and
# ts/src/stdlib/sources.ts, go/alchemy.go and go/stdlib/. Edit the root
# files, then run this; never edit a copy. `node ts/embed-grammar.js
# --check` compares without writing.
embed:
	cd ts && npm run embed

# Set the Rust crate version: make version-rs V=x.y.z
#
# Bumps rs/Cargo.toml's version and the crate's own entry in
# rs/Cargo.lock (rs/src/lib.rs's VERSION reads Cargo.toml at build time).
# It neither commits nor tags: the release workflow publishes the crate
# from the release tag. ts/test/version.test.ts and go/version_test.go
# hold the TypeScript and Go versions to this one, and ci/rust/run.sh the
# lock's entry to the manifest.
version-rs:
	@test -n "$(V)" || (echo "Usage: make version-rs V=x.y.z" && exit 1)
	sed -i.bak 's/^version = ".*"/version = "$(V)"/' rs/Cargo.toml
	rm -f rs/Cargo.toml.bak
	cd rs && cargo metadata --format-version 1 --offline >/dev/null

# Reinstall and rebuild from scratch. `npm i` installs the published
# @tabnas packages, so re-run admin/scripts/link.sh afterwards to test
# against the sibling checkouts.
reset:
	cd ts && npm run reset
	cd go && go clean -cache && go build ./... && go test -count=1 -v ./...
