// Copyright (c) 2026 tabnas, MIT License

// Command embed copies the shared sources into the Go module: the grammar
// document, alchemy-grammar.jsonic, between the
// `--- BEGIN/END EMBEDDED alchemy-grammar.jsonic ---` markers of
// go/alchemy.go as a raw string, and stdlib/*.alc into go/stdlib/ for
// go:embed. Never hand-edit a copy: edit the root file and run, from go/,
//
//	go run ./internal/embed
//
// With -check it writes nothing and exits 1 naming each copy that differs
// from its source. The module's own tests (shared_sources_test.go) fail
// the same way, so a forgotten embed is red either way.
//
// This is the Go target of the repository's embed step. A Go raw string
// has no escapes and cannot hold a backquote, and the grammar's comments
// quote names in backquotes, so each backquote is spliced in as an
// interpreted "`" between two raw strings; the constant's value is the
// file's text exactly, which is what the drift test compares. The
// literal, for an embedder in another language, is
//
//	"const grammarText = `\n" + text.split("`").join("` + \"`\" + `") + "`\n\n"
package main

import (
	"flag"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
)

const (
	begin = "// --- BEGIN EMBEDDED alchemy-grammar.jsonic ---"
	end   = "// --- END EMBEDDED alchemy-grammar.jsonic ---"
)

// GoLiteral is the Go source of the constant holding text.
func GoLiteral(text string) string {
	return "const grammarText = `\n" + strings.ReplaceAll(text, "`", "` + \"`\" + `") + "`\n\n"
}

func main() {
	check := flag.Bool("check", false, "compare only; exit 1 when a copy differs")
	flag.Parse()

	// Run from go/; the repository root is its parent.
	root := ".."
	drift := 0
	write := func(path, text string) {
		was, err := os.ReadFile(path)
		if err == nil && string(was) == text {
			return
		}
		if *check {
			fmt.Fprintf(os.Stderr, "%s differs from its source: run go run ./internal/embed\n", path)
			drift++
			return
		}
		if err := os.WriteFile(path, []byte(text), 0o644); err != nil {
			fmt.Fprintln(os.Stderr, err)
			os.Exit(2)
		}
		fmt.Println("embedded", path)
	}

	grammar, err := os.ReadFile(filepath.Join(root, "alchemy-grammar.jsonic"))
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(2)
	}
	target := "alchemy.go"
	src, err := os.ReadFile(target)
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(2)
	}
	s := string(src)
	b, e := strings.Index(s, begin), strings.Index(s, end)
	if b < 0 || e < 0 || e < b {
		fmt.Fprintln(os.Stderr, "embed markers not found in", target)
		os.Exit(2)
	}
	write(target, s[:b]+begin+"\n"+GoLiteral(string(grammar))+s[e:])

	sources, err := filepath.Glob(filepath.Join(root, "stdlib", "*.alc"))
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(2)
	}
	sort.Strings(sources)
	if err := os.MkdirAll("stdlib", 0o755); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(2)
	}
	names := map[string]bool{}
	for _, path := range sources {
		text, err := os.ReadFile(path)
		if err != nil {
			fmt.Fprintln(os.Stderr, err)
			os.Exit(2)
		}
		names[filepath.Base(path)] = true
		write(filepath.Join("stdlib", filepath.Base(path)), string(text))
	}
	copies, _ := filepath.Glob(filepath.Join("stdlib", "*.alc"))
	for _, path := range copies {
		if names[filepath.Base(path)] {
			continue
		}
		if *check {
			fmt.Fprintf(os.Stderr, "%s has no source in stdlib/: run go run ./internal/embed\n", path)
			drift++
			continue
		}
		if err := os.Remove(path); err != nil {
			fmt.Fprintln(os.Stderr, err)
			os.Exit(2)
		}
		fmt.Println("removed", path)
	}
	if drift > 0 {
		os.Exit(1)
	}
	if *check {
		fmt.Println("embedded copies match their sources")
	}
}
