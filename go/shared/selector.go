// Copyright (c) 2026 tabnas, MIT License

package shared

import (
	"strconv"
	"strings"
)

// Segment is one step of a concrete path: a member Key, or an element
// Index when IsIndex.
type Segment struct {
	Key     string
	Index   int
	IsIndex bool
}

// KeySegment is a member step.
func KeySegment(key string) Segment { return Segment{Key: key} }

// IndexSegment is an element step.
func IndexSegment(i int) Segment { return Segment{Index: i, IsIndex: true} }

// Path is a concrete location in a document. It prints in jq syntax.
type Path []Segment

// String is the path in jq syntax: `.` for the root, `.a[0]."b-c"`.
func (p Path) String() string {
	if len(p) == 0 {
		return "."
	}
	var b strings.Builder
	for _, seg := range p {
		if seg.IsIndex {
			b.WriteByte('[')
			b.WriteString(strconv.Itoa(seg.Index))
			b.WriteByte(']')
		} else {
			writeKey(&b, seg.Key)
		}
	}
	return b.String()
}

// Clone is a copy that shares nothing with p.
func (p Path) Clone() Path { return append(Path(nil), p...) }

// writeKey writes one key as jq does: bare when it is an ASCII letter or
// `_` followed by ASCII letters, digits and `_`, quoted as a JSON string
// otherwise.
func writeKey(b *strings.Builder, key string) {
	b.WriteByte('.')
	if bareKey(key) {
		b.WriteString(key)
		return
	}
	writeJSONString(key, b)
}

func bareKey(key string) bool {
	if key == "" {
		return false
	}
	for i := 0; i < len(key); i++ {
		c := key[i]
		alpha := (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || c == '_'
		if !alpha && (i == 0 || c < '0' || c > '9') {
			return false
		}
	}
	return true
}

// StepKind names one kind of selector step.
type StepKind uint8

const (
	// StepProperty is the member with a name, inside an object.
	StepProperty StepKind = iota
	// StepIndex is the element at a position, inside an array.
	StepIndex
	// StepEachIndex is every element of an array.
	StepEachIndex
	// StepEachMember is every member value of an object.
	StepEachMember
)

// Step is one step of a selector: Name for StepProperty, Index for
// StepIndex.
type Step struct {
	Kind  StepKind
	Name  string
	Index int
}

// Selector is a description of locations: the root, narrowed step by
// step. A selector is data, never code: it is built from constructors or
// from validated path segments (FromSegments), and a matcher interprets
// it. The zero Selector is the root. The builder methods return a new
// selector and never change the one they are called on.
type Selector struct {
	steps []Step
}

// Root is the document itself.
func Root() Selector { return Selector{} }

// SelectorOf is a selector of the given steps.
func SelectorOf(steps ...Step) Selector { return Selector{steps: append([]Step(nil), steps...)} }

func (s Selector) with(step Step) Selector {
	steps := make([]Step, len(s.steps), len(s.steps)+1)
	copy(steps, s.steps)
	return Selector{steps: append(steps, step)}
}

// Property narrows to the member with this name.
func (s Selector) Property(name string) Selector { return s.with(Step{Kind: StepProperty, Name: name}) }

// Index narrows to the element at this position.
func (s Selector) Index(i int) Selector { return s.with(Step{Kind: StepIndex, Index: i}) }

// EachIndex narrows to every element.
func (s Selector) EachIndex() Selector { return s.with(Step{Kind: StepEachIndex}) }

// EachMember narrows to every member value.
func (s Selector) EachMember() Selector { return s.with(Step{Kind: StepEachMember}) }

// Compose is s, then other below every location s names.
func (s Selector) Compose(other Selector) Selector {
	steps := make([]Step, 0, len(s.steps)+len(other.steps))
	steps = append(steps, s.steps...)
	return Selector{steps: append(steps, other.steps...)}
}

// FromSegments is a selector naming exactly one location: `as-path`
// over data.
func FromSegments(segments []Segment) Selector {
	steps := make([]Step, len(segments))
	for i, seg := range segments {
		if seg.IsIndex {
			steps[i] = Step{Kind: StepIndex, Index: seg.Index}
		} else {
			steps[i] = Step{Kind: StepProperty, Name: seg.Key}
		}
	}
	return Selector{steps: steps}
}

// Steps is the selector's steps; the caller must not change them.
func (s Selector) Steps() []Step { return s.steps }

// IsRoot reports whether the selector names the document itself.
func (s Selector) IsRoot() bool { return len(s.steps) == 0 }

// IsMulti reports whether the selector can name more than one location.
func (s Selector) IsMulti() bool {
	for _, st := range s.steps {
		if st.Kind == StepEachIndex || st.Kind == StepEachMember {
			return true
		}
	}
	return false
}

// Equal reports whether two selectors have the same steps.
func (s Selector) Equal(o Selector) bool {
	if len(s.steps) != len(o.steps) {
		return false
	}
	for i := range s.steps {
		if s.steps[i] != o.steps[i] {
			return false
		}
	}
	return true
}

// MayOverlap reports whether this selector may name a location strictly
// inside a location other names, or the same one: the conservative test
// a router uses to refuse overlapping captures.
func (s Selector) MayOverlap(other Selector) bool {
	short, long := s.steps, other.steps
	if len(short) > len(long) {
		short, long = long, short
	}
	for i := range short {
		if !stepMayMatchSame(short[i], long[i]) {
			return false
		}
	}
	return true
}

func stepMayMatchSame(a, b Step) bool {
	switch {
	case a.Kind == StepProperty && b.Kind == StepProperty:
		return a.Name == b.Name
	case a.Kind == StepIndex && b.Kind == StepIndex:
		return a.Index == b.Index
	}
	member := func(k StepKind) bool { return k == StepProperty || k == StepEachMember }
	index := func(k StepKind) bool { return k == StepIndex || k == StepEachIndex }
	return (member(a.Kind) && member(b.Kind)) || (index(a.Kind) && index(b.Kind))
}

// String is the selector in jq syntax: `.response.records[*]`,
// `."odd key"`, `[3]`, `[]` for every member, `.` for the root.
func (s Selector) String() string {
	if len(s.steps) == 0 {
		return "."
	}
	var b strings.Builder
	for _, st := range s.steps {
		switch st.Kind {
		case StepProperty:
			writeKey(&b, st.Name)
		case StepIndex:
			b.WriteByte('[')
			b.WriteString(strconv.Itoa(st.Index))
			b.WriteByte(']')
		case StepEachIndex:
			b.WriteString("[*]")
		case StepEachMember:
			b.WriteString("[]")
		}
	}
	return b.String()
}
