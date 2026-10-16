// Copyright (c) 2026 tabnas, MIT License

package shared

import "testing"

// The fixture encoding of a parser event round-trips, leaving an absent
// field out, and a malformed one says what is wrong with it.
func TestParserEventEncodingRoundTrips(t *testing.T) {
	texts := []string{
		`["open",{"name":"map","d":1,"i":3,"alt":{"p":"pair","r":"","b":0,"g":["map","json"]},"tokens":[["#OB","{","{",1,1]],"forced":false,"n":{"pk":0}}]`,
		`["close",{"name":"val","d":2,"i":4,"tokens":[],"forced":true,"n":{},"value":null}]`,
		`["end"]`,
	}
	for _, text := range texts {
		ev, err := ParserEventFromJSON(text)
		if err != nil {
			t.Fatalf("%s: %v", text, err)
		}
		if got := ev.JSON(); got != text {
			t.Errorf("round trip:\n  got  %s\n  want %s", got, text)
		}
	}
	open, _ := ParserEventFromJSON(texts[0])
	if open.Kind != ParserOpen || open.Pass.Value != nil || open.Pass.Alt.P != "pair" {
		t.Errorf("an open pass has no value: %+v", open.Pass)
	}
	for _, text := range []string{
		`{"open":1}`,
		`["start"]`,
		`["open"]`,
		`["close",{"name":"x","d":-1,"i":0,"tokens":[],"forced":false,"n":{}}]`,
		`["close",{"name":"x","d":1,"i":0,"tokens":[["#ST"]],"forced":false,"n":{}}]`,
	} {
		if _, err := ParserEventFromJSON(text); err == nil {
			t.Errorf("%s: no error", text)
		}
	}
}
