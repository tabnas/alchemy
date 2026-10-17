//! The spec of the library's `transduce-tree`, for routers that run it
//! natively.
//!
//! `transduce-tree spec input` (`stdlib/transduce.alc`) makes a tree's
//! JSON events of a grammar's rule passes as `spec` says: a record keyed
//! by rule name, each entry a record of what that rule's passes do. The
//! library's text is the specification. A host's routers may run it
//! natively ([`crate::shared::Routers::transduce_tree`]), and the lowering
//! asks them to only when the program's spec has the standard shape
//! [`TreeSpec`] holds: every entry a record whose `:container` is
//! `:object` or `:array`, whose `:key` and `:member` are each a string or
//! a token's index, whose `:scalar` is a token's index or `:value`, and
//! whose `:text` is a boolean, each of the five left out at will. A spec
//! of any other shape runs the library's text, which says what it does
//! with it.
//!
//! What a native must do is what the text does, pass for pass:
//!
//! - An open pass of a rule the spec names announces its `:key` first:
//!   the string, or the `:val` of the pass's token at that index, nothing
//!   when the pass consumed no such token.
//! - It then begins its `:container`, unless the rule it replaced carried
//!   that container on (below), or yields its `:scalar` from the token at
//!   that index, when the pass consumed one; `:value` yields nothing on an
//!   open pass, and `:scalar` nothing for a container rule.
//! - It then announces its `:member`, read as `:key` is.
//! - A close pass of a rule whose `:text` is `true` first yields the text
//!   the passes between its open pass and it consumed: the `:src` of each
//!   of their tokens, in order, the tokens of any `:text` rule closed in
//!   between included. Every pass writes its tokens' `:src` to the text
//!   of the innermost `:text` rule open, if one is: an open pass before
//!   its own rule's text opens, a close pass after its own rule's text
//!   closes and joins the one around it.
//! - A close pass of a container rule then ends its container, unless the
//!   pass's alternate replaces the rule with one the spec names a
//!   container: then the container goes on, and the replacing rule's open
//!   pass begins none. A close pass of any other rule yields its
//!   `:scalar :value`, the pass's own value, when it has one.
//! - A pass of a rule the spec does not name yields nothing but writes
//!   its tokens as above, and the end of the passes inside a container is
//!   `PROTOCOL_ORDER_ERROR`; a `:text` rule's text still open there is
//!   dropped.
//!
//! The library's failures are the native's, with their codes and texts: a
//! key or a member whose token's value is not a string, and a scalar whose
//! token's value is a container, are `DSL_TYPE_ERROR`; a close with no
//! container open is the library's `pop` of an empty stack, and a `:text`
//! rule's close with no text open its `top` of an empty one, each
//! `DSL_TYPE_ERROR` too. A text is held to [`TreeSpec::max_text_bytes`] as
//! it grows, as the library's `string-join` holds it: a pass's tokens
//! together, then the text they join, then a closed text joined to the one
//! around it, each past the bound refused with [`tree_text_refusal`]; a pass
//! that consumed no tokens joins nothing. (The library's scan also holds
//! its whole state to `max_metadata_bytes`, which a native has no state to
//! match: past that, the two refuse with the same code and different
//! limits.)
//!
//! A router built before `:member` and `:text` existed reads neither, so
//! the lowering hands a spec that uses either only to routers that say
//! they do ([`crate::shared::Routers::transduce_tree_reads_member_and_text`]).

use std::collections::HashMap;

use crate::shared::error::Fail;

/// The container a rule's passes begin and end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeContainer {
    Object,
    Array,
}

/// The member name a rule's open pass announces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TreeKey {
    /// The `:val` of the pass's token at this index.
    Token(usize),
    /// This name.
    Name(Box<str>),
}

/// The scalar a rule yields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeScalar {
    /// The `:val` of the open pass's token at this index.
    Token(usize),
    /// The close pass's own value.
    Value,
}

/// What one rule's passes do: an entry of the spec.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TreeRule {
    pub container: Option<TreeContainer>,
    pub key: Option<TreeKey>,
    /// Read only when `container` is `None`, as the library reads it.
    pub scalar: Option<TreeScalar>,
    /// The member name the open pass announces after what it begins or
    /// yields.
    pub member: Option<TreeKey>,
    /// Whether the close pass first yields the text the passes in between
    /// consumed.
    pub text: bool,
}

/// A `transduce-tree` spec of the standard shape: the rules it names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeSpec {
    pub rules: HashMap<Box<str>, TreeRule>,
    /// The most bytes a `:text` rule's text may hold as it grows: the
    /// runtime's `max_scalar_bytes`, which the library's `string-join`
    /// holds it to. The lowering sets it when it hands the spec to a
    /// native; unbounded until then.
    pub max_text_bytes: usize,
}

impl Default for TreeSpec {
    fn default() -> TreeSpec {
        TreeSpec {
            rules: HashMap::new(),
            max_text_bytes: usize::MAX,
        }
    }
}

impl TreeSpec {
    pub fn new() -> TreeSpec {
        TreeSpec::default()
    }

    /// The spec with its texts held to `max` bytes as they grow.
    pub fn with_max_text_bytes(mut self, max: usize) -> TreeSpec {
        self.max_text_bytes = max;
        self
    }

    /// The spec with `rule`'s entry set.
    pub fn with(mut self, name: &str, rule: TreeRule) -> TreeSpec {
        self.rules.insert(name.into(), rule);
        self
    }

    /// The entry of the rule named `name`, when the spec names it.
    pub fn rule(&self, name: &str) -> Option<&TreeRule> {
        self.rules.get(name)
    }

    /// Whether the rule named `name` is a container rule: what decides,
    /// at a close whose alternate replaces its rule with that one, that
    /// the container goes on.
    pub fn is_container(&self, name: &str) -> bool {
        self.rule(name).is_some_and(|r| r.container.is_some())
    }

    /// Whether any rule names a `:member` or is a `:text` rule: what a
    /// router built before those fields cannot read.
    pub fn uses_member_or_text(&self) -> bool {
        self.rules.values().any(|r| r.member.is_some() || r.text)
    }

    /// The spec as the library's `transduce-step` reads it: without the
    /// `:member` and `:text` of its entries, which only `transduce-tree`'s
    /// own step, `transduce-pass`, reads.
    pub fn without_member_and_text(mut self) -> TreeSpec {
        for rule in self.rules.values_mut() {
            rule.member = None;
            rule.text = false;
        }
        self
    }
}

/// The library's refusal of a text past [`TreeSpec::max_text_bytes`]: its
/// `string-join`'s, `len` the bytes the join would hold.
pub fn tree_text_refusal(len: usize, max: usize) -> Fail {
    Fail::limit(
        "max_scalar_bytes",
        max as u64,
        format!("string-join: the joined string is {len} bytes, more than {max}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_spec_answers_by_rule_name() {
        let spec = TreeSpec::new()
            .with(
                "map",
                TreeRule {
                    container: Some(TreeContainer::Object),
                    ..TreeRule::default()
                },
            )
            .with(
                "pair",
                TreeRule {
                    key: Some(TreeKey::Token(0)),
                    ..TreeRule::default()
                },
            );
        assert!(spec.is_container("map"));
        assert!(!spec.is_container("pair"));
        assert!(!spec.is_container("val"));
        assert_eq!(spec.rule("pair").unwrap().key, Some(TreeKey::Token(0)));
        assert_eq!(spec.rule(""), None);
    }
}
