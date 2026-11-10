// Copyright (c) 2026 tabnas, MIT License

package shared

import "testing"

// The fixture encoding of a parser event round-trips, leaving an absent
// field out, and a malformed one says what is wrong with it.
func TestParserEventEncodingRoundTrips(t *testing.T) {
	texts := []string{
		`["open",{"name":"map","d":1,"i":3,"alt":{"p":"pair","r":"","b":0,"g":["map","json"]},"tokens":[["#OB","{","{",1,1,0]],"forced":false,"n":{"pk":0}}]`,
		`["close",{"name":"val","d":2,"i":4,"tokens":[],"forced":true,"n":{},"value":null}]`,
		`["token",{"name":"#CM","val":"/* é */","src":"/* é */","row":2,"col":3,"offset":7}]`,
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
	if open.Pass.Tokens[0].Offset != 0 {
		t.Errorf("a pass's token carries its offset: %+v", open.Pass.Tokens[0])
	}
	token, _ := ParserEventFromJSON(texts[2])
	if token.Kind != ParserToken || token.Token.Name != "#CM" || token.Token.Offset != 7 || token.Pass != nil {
		t.Errorf("a token item: %+v", token)
	}
	for _, text := range []string{
		`{"open":1}`,
		`["start"]`,
		`["open"]`,
		`["close",{"name":"x","d":-1,"i":0,"tokens":[],"forced":false,"n":{}}]`,
		`["close",{"name":"x","d":1,"i":0,"tokens":[["#ST"]],"forced":false,"n":{}}]`,
		`["close",{"name":"x","d":1,"i":0,"tokens":[["#ST","a","a",1,1]],"forced":false,"n":{}}]`,
		`["token"]`,
		`["token",{"name":"#SP","val":" ","src":" ","row":1,"col":1}]`,
		`["token",{"name":"#SP","val":" ","src":" ","row":1,"col":1,"offset":-1}]`,
	} {
		if _, err := ParserEventFromJSON(text); err == nil {
			t.Errorf("%s: no error", text)
		}
	}
}

// A sink takes no token items unless it asks.
func TestASinkTakesNoTokenItemsUnlessItAsks(t *testing.T) {
	if WantsTokens(silent{}) || WantsTokens(asking{false}) || !WantsTokens(asking{true}) {
		t.Errorf("WantsTokens answers what the sink asks, and false for a sink that cannot ask")
	}
}

type silent struct{}

func (silent) ParserEvent(ParserEvent) (Flow, *Fail) { return Continue, nil }

type asking struct{ wants bool }

func (asking) ParserEvent(ParserEvent) (Flow, *Fail) { return Continue, nil }
func (a asking) WantsTokens() bool                   { return a.wants }
