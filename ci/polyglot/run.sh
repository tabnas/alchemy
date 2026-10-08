#!/usr/bin/env bash
# TypeScript and Go gate. It builds each TypeScript sibling before linking it
# into alchemy, and uses a temporary Go workspace so the Go modules resolve to
# the same sibling checkouts rather than to their published versions. The
# tests that run programs on transduce and render are alchemy-cli's, which
# .github/workflows/downstream.yml runs against this checkout.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
FLEET_ROOT=$(cd "$ROOT/.." && pwd)

COMMON_SIBLINGS=$(sed -n 's/^SIBLINGS="\(.*\)"$/\1/p' "$ROOT/ci/rust/run.sh")
TS_ONLY_SIBLINGS=$(sed -n 's/^TS_SIBLINGS="\(.*\)"$/\1/p' "$ROOT/ci/rust/run.sh")
# Dependencies first: debug builds on bnf, abnf and railroad, and alchemy last.
TS_PACKAGES="parser support bnf abnf railroad debug json alchemy"
ALL_SIBLINGS="$COMMON_SIBLINGS $TS_ONLY_SIBLINGS"

for sibling in $ALL_SIBLINGS; do
  if [[ ! -f "$FLEET_ROOT/$sibling/ts/package.json" ]]; then
    echo "no $sibling TypeScript checkout at $FLEET_ROOT/$sibling/ts" >&2
    exit 1
  fi
done

link_siblings() {
  local package_dir=$1 sibling name target
  for sibling in $ALL_SIBLINGS; do
    [[ "$FLEET_ROOT/$sibling/ts" != "$package_dir" ]] || continue
    name=$(cd "$FLEET_ROOT/$sibling/ts" && node -p "require('./package.json').name")
    target="$package_dir/node_modules/$name"
    [[ -e "$target" || -L "$target" ]] || continue
    rm -rf "$target"
    mkdir -p "$(dirname "$target")"
    ln -s "$FLEET_ROOT/$sibling/ts" "$target"
  done
}

package_i=0
package_total=$(wc -w <<<"$TS_PACKAGES")
for package in $TS_PACKAGES; do
  package_i=$((package_i + 1))
  package_dir="$FLEET_ROOT/$package/ts"
  echo "typescript: $package_i of $package_total ($((package_i * 100 / package_total))%) install $package"
  (cd "$package_dir" && npm install --ignore-scripts --no-save)
  link_siblings "$package_dir"
  echo "typescript: $package_i of $package_total ($((package_i * 100 / package_total))%) build $package"
  (cd "$package_dir" && npm run build --if-present)
done

echo "typescript: test alchemy (100%)"
(cd "$ROOT/ts" && npm test)

WORK_DIR=$(mktemp -d)
trap 'rm -rf "$WORK_DIR"' EXIT
(
  cd "$WORK_DIR"
  go work init
  for sibling in $COMMON_SIBLINGS alchemy; do
    module="$FLEET_ROOT/$sibling/go"
    [[ -f "$module/go.mod" ]] || continue
    go work use "$module"
  done
)

echo "go: test alchemy (50%)"
(cd "$ROOT/go" && GOWORK="$WORK_DIR/go.work" go test -count=1 ./...)
echo "go: vet alchemy (100%)"
(cd "$ROOT/go" && GOWORK="$WORK_DIR/go.work" go vet ./...)
echo "polyglot gate: green"
