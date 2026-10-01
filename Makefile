# Build and test the Rust crate in rs/. (TypeScript and Go ports are not
# started yet; their targets join here when they are.)

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
	ci/rust/run.sh

clean:
	cd rs && cargo clean
