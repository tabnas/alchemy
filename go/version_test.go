// Copyright (c) 2026 tabnas, MIT License

package tabnasalchemy

// version_test.go: the baked-in VERSION must equal the version the
// repository's reference implementation declares, rs/Cargo.toml's
// [package] version. The release rewrites both; a release that bumps one
// and forgets the other fails here instead of shipping a lie.

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestVersionMatchesTheCrate(t *testing.T) {
	raw, err := os.ReadFile(filepath.Join("..", "rs", "Cargo.toml"))
	if err != nil {
		// Deliberately fatal, never skipped: a version check that silently
		// does not run is the failure mode this test exists to prevent.
		t.Fatalf("cannot read rs/Cargo.toml, so VERSION cannot be checked: %v", err)
	}
	inPackage := false
	version := ""
	for _, line := range strings.Split(string(raw), "\n") {
		line = strings.TrimSpace(line)
		if strings.HasPrefix(line, "[") {
			inPackage = line == "[package]"
			continue
		}
		if inPackage && strings.HasPrefix(line, "version") {
			if i := strings.Index(line, "\""); i >= 0 {
				version = strings.Trim(line[i:], "\"")
			}
			break
		}
	}
	if version == "" {
		t.Fatal("rs/Cargo.toml has no [package] version")
	}
	if VERSION != version {
		t.Errorf("VERSION drift: go VERSION = %q but rs/Cargo.toml = %q", VERSION, version)
	}
}
