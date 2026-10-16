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
//! `:object` or `:array`, whose `:key` is a string or a token's index,
//! and whose `:scalar` is a token's index or `:value`, each of the three
//! left out at will. A spec of any other shape runs the library's text,
//! which says what it does with it.
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
//! - A close pass of a container rule ends its container, unless the
//!   pass's alternate replaces the rule with one the spec names a
//!   container: then the container goes on, and the replacing rule's open
//!   pass begins none. A close pass of any other rule yields its
//!   `:scalar :value`, the pass's own value, when it has one.
//! - A pass of a rule the spec does not name yields nothing, and the end
//!   of the passes inside a container is `PROTOCOL_ORDER_ERROR`.
//!
//! The library's failures are the native's, with their codes and texts: a
//! key whose token's value is not a string, and a scalar whose token's
//! value is a container, are `DSL_TYPE_ERROR`; a close with no container
//! open is the library's `pop` of an empty stack, `DSL_TYPE_ERROR` too.

use std::collections::HashMap;

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
}

/// A `transduce-tree` spec of the standard shape: the rules it names.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TreeSpec {
    pub rules: HashMap<Box<str>, TreeRule>,
}

impl TreeSpec {
    pub fn new() -> TreeSpec {
        TreeSpec::default()
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
