// Copyright (c) 2026 tabnas, MIT License

package tabnasalchemy

// lower_test.go: plans to sinks (rs/src/lower.rs's tests): how a csv
// plan's options map onto the renderer's dialect. The tests that lower a
// program to a sink and run it are alchemy-cli's (go/e2e/lower_test.go):
// they need transduce's routers and render's renderers.

import (
	"testing"

	"github.com/tabnas/alchemy/go/shared"
)

func TestTheCsvOptionsMapToTheDialectOrNot(t *testing.T) {
	rt := runtimeOf(t, "def o csv-options\ndef lf (record (entry :delimiter \";\") (entry :newline \"\\n\") (entry :header false) (entry :null-text \"NULL\") (entry :missing \"-\"))\ndef bad (record (entry :delimiter \"ab\"))")
	value := func(name string) Val {
		v, f := rt.DefValue(ScopeProgram, name)
		if f != nil || v == nil {
			t.Fatalf("%s: %v", name, f)
		}
		return v
	}
	if o, ok := csvOptions(value("o")); !ok || o.Delimiter != ',' || o.Newline != shared.NewlineCRLF || !o.Header || o.NullText != "" || o.Missing != nil || o.Quoting != shared.QuotingAlways {
		t.Errorf("%+v %v", o, ok)
	}
	lf, ok := csvOptions(value("lf"))
	if !ok || lf.Delimiter != ';' || lf.Newline != shared.NewlineLF || lf.Header || lf.NullText != "NULL" || lf.Missing == nil || *lf.Missing != "-" {
		t.Errorf("%+v %v", lf, ok)
	}
	if _, ok := csvOptions(value("bad")); ok {
		t.Error("a two-character delimiter maps")
	}
	if _, ok := csvOptions(NullVal{}); ok {
		t.Error("null maps")
	}
}
