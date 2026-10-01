// Copyright (c) 2026 tabnas, MIT License

package tabnasalchemy

// shared_sources_test.go: the shared sources every runtime embeds, held to
// the files at the repository root (rs/tests/shared_sources_test.rs).
// alchemy-grammar.jsonic is carried between the BEGIN/END EMBEDDED markers
// of alchemy.go, and stdlib/*.alc as the module-local copies in go/stdlib/
// that stdlib.go embeds; `make embed` (ts/embed-grammar.js) writes both.
// This fails when a copy and its source differ, or a file is in one place
// and not the other, so a forgotten embed is red rather than a quiet
// drift.

import (
	"os"
	"path/filepath"
	"sort"
	"strings"
	"testing"
)

func TestTheEmbeddedGrammarIsTheAuthoredOne(t *testing.T) {
	authored, err := os.ReadFile(filepath.Join(repoRoot(t), "alchemy-grammar.jsonic"))
	if err != nil {
		t.Fatal(err)
	}
	// The literal opens with the line feed the embedder writes first.
	if GrammarText() != "\n"+string(authored) {
		t.Fatal("alchemy.go embeds a different grammar from alchemy-grammar.jsonic: run make embed")
	}
}

func alcFiles(t *testing.T, dir string) []string {
	t.Helper()
	entries, err := os.ReadDir(dir)
	if err != nil {
		t.Fatal(err)
	}
	var out []string
	for _, e := range entries {
		if strings.HasSuffix(e.Name(), ".alc") {
			out = append(out, e.Name())
		}
	}
	sort.Strings(out)
	return out
}

func TestTheEmbeddedStdlibIsEveryFile(t *testing.T) {
	root := repoRoot(t)
	canonical := alcFiles(t, filepath.Join(root, "stdlib"))
	packaged := alcFiles(t, "stdlib")
	if len(canonical) == 0 {
		t.Fatal("stdlib/ holds the library")
	}
	if strings.Join(packaged, " ") != strings.Join(canonical, " ") {
		t.Fatalf("go/stdlib/ holds %v and stdlib/ %v: run make embed", packaged, canonical)
	}
	// The module loads every file, by the name its spans carry.
	var embedded []string
	for _, file := range StdlibFiles {
		embedded = append(embedded, strings.TrimPrefix(file, "stdlib/"))
	}
	sort.Strings(embedded)
	if strings.Join(embedded, " ") != strings.Join(canonical, " ") {
		t.Fatalf("StdlibFiles %v does not name every stdlib/*.alc %v", embedded, canonical)
	}
	for _, file := range StdlibFiles {
		source, err := os.ReadFile(filepath.Join(root, file))
		if err != nil {
			t.Fatal(err)
		}
		text, ok := StdlibSource(file)
		if !ok || text != string(source) {
			t.Errorf("go/%s is not %s: run make embed", file, file)
		}
	}
}
