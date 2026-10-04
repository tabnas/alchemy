// Copyright (c) 2026 tabnas, MIT License

package shared

// Transition is the result of one scan-emit step: the next state and
// what to emit for it.
type Transition[S, O any] struct {
	State   S
	Outputs []O
}
