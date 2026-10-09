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

// :non-finite and :no-columns, when the record has them, are policies,
// which the lowering applies around the renderer; one that is none maps to
// no dialect, and the library's csv runs, which refuses it where it is
// read.
func TestACsvOptionsRecordMapsWithANonFinitePolicyAndNotWithAnotherValue(t *testing.T) {
	const base = `(entry :delimiter ",") (entry :newline "\r\n") (entry :header true) (entry :null-text "") (entry :missing :error)`
	record := func(policy string) string { return "(record " + base + " " + policy + ")" }
	for _, c := range []struct {
		policy string
		maps   bool
	}{
		{"(entry :non-finite :literal)", true}, {"(entry :non-finite :null)", true}, {"(entry :non-finite :reject)", true},
		{"(entry :non-finite :nope)", false}, {"(entry :non-finite 1)", false},
		{"(entry :no-columns :empty)", true}, {"(entry :no-columns :refuse)", true}, {"(entry :no-columns :nope)", false},
	} {
		o, ok := csvOptions(mustEval(t, "", record(c.policy)))
		if ok != c.maps || (ok && o != shared.DefaultCSVOptions()) {
			t.Errorf("%s: %+v %v", c.policy, o, ok)
		}
		// The fast path takes what maps, and declines the rest for the
		// library's text.
		src := "def b\n  record\n    entry :columns (path \"m\")\n    entry :rows (path \"r\" each-index)\n    entry :column (fn [d] d)\n" +
			"def export [input] (csv " + record(c.policy) + " (table-from-json b input))"
		text, ok := mustCompile(t, src, "t.alc").Result().(TextVal)
		if !ok {
			t.Errorf("%s: not a text", c.policy)
		} else if (text.Plan.Kind == PlanCsv) != c.maps {
			t.Errorf("%s: %s", c.policy, PlanName(text.Plan))
		}
	}
}
