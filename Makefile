# Build and test the Rust crate in rs/. The full gate below also runs the
# TypeScript and Go structural-conformance suites against sibling checkouts.

.PHONY: all build test clean bench gate embed

all: build test

build:
	cd rs && cargo build --all-targets

test:
	cd rs && cargo test --all-targets
	cd rs && cargo test --doc
	cd rs && cargo clippy --all-targets --all-features -- -D warnings

bench:
	cd rs && cargo bench

# Copy the shared sources (alchemy-grammar.jsonic, stdlib/*.alc) into
# every runtime. Edit the root files, then run this; never edit a copy.
embed:
	node scripts/embed.js

# The full gate CI runs, with the lock discipline.
gate:
	ci/polyglot/run.sh
	ci/rust/run.sh

clean:
	cd rs && cargo clean
