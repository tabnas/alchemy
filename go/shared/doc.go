// Copyright (c) 2026 tabnas, MIT License

// Package shared holds the types tabnas-alchemy shares with the packages
// a program runs on: the protocols tabnas-transduce produces (JsonEvents/1
// events and the Sink they are pushed into, TableRows/1, selectors,
// retained values, limits, metrics, the abort flag, and the stable
// failure Codes every stage reports), the types tabnas-render's renderers
// face (TextOut and the CSV and JSON options), and the two interfaces
// through which alchemy's runtime is handed its routers and renderers,
// Routers and Renderers.
//
// The package imports nothing from the fleet. Transduce and render depend
// on it, never the reverse: each keeps its own names for these types, as
// aliases, so its API is unchanged, and each answers an implementation of
// one of the interfaces (transduce's Routers, render's Renderers), which
// a host passes to alchemy's Compile.
package shared
