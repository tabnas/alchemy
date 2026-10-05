// Copyright (c) 2026 tabnas, MIT License

package shared

// JSONOptions is the JSON profile.
type JSONOptions struct {
	// Indent is spaces per nesting level, with a newline before every
	// item and every closing bracket of a non-empty container. Zero (or
	// less) is compact: no whitespace at all.
	Indent int
	// TrailingNewline writes a newline after the root value, at End.
	TrailingNewline bool
}
