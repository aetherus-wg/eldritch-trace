//! Pattern matching against the Trie

use std::collections::HashSet;

use anyhow::Result;
use log::{debug, trace};

use crate::{
    SrcId,
    trie::{self, Trie, TrieNode},
};
use et_core::bits::BitsMatch;

/// Field type: Named field, SrcId type or don't care (X)
#[derive(Debug)]
pub enum Field<'a> {
    /// Don't care
    X,
    /// SrcId type field: MatId, SurfId, etc.
    SrcId(SrcId),
    /// Named field: MCRT, Material, Elastic, etc.
    Field(&'a str),
}

/// Pattern is a sequence of fields that we want to match against the Trie
#[derive(Debug)]
pub struct Pattern<'a>(pub Vec<Field<'a>>);

/// Search the Trie for the given pattern and return the combined BitsMatch and SrcId if a match is found
pub fn search_trie(trie: &Trie, pattern: &Pattern) -> Result<(BitsMatch, SrcId)> {
    let mut encodings = Vec::new();
    #[derive(Clone, Hash, PartialEq, Eq)]
    struct StackEntry<'a> {
        node:          &'a TrieNode,
        pattern_index: usize,
        prev_trie_x:   bool,
        bits_match:    BitsMatch,
        encoding:      Vec<String>,
        src_id:        SrcId,
    }
    impl<'a> StackEntry<'a> {
        pub fn step_pattern(&self) -> StackEntry<'a> {
            StackEntry {
                node:          self.node,
                pattern_index: self.pattern_index + 1,
                prev_trie_x:   self.prev_trie_x,
                bits_match:    self.bits_match.clone(),
                encoding:      self.encoding.clone(),
                src_id:        self.src_id.clone(),
            }
        }
        pub fn step_trie(
            &self,
            child_field: &trie::Field,
            child_node: &'a trie::TrieNode,
        ) -> StackEntry<'a> {
            StackEntry {
                node:          child_node,
                pattern_index: self.pattern_index,
                prev_trie_x:   matches!(child_field, trie::Field::X { attr: None, .. }),
                bits_match:    self.bits_match.combine(child_field.bits_match()),
                encoding: {
                    let mut encoding = self.encoding.clone();
                    encoding.push(child_field.to_string());
                    encoding
                },
                src_id: self.src_id.clone(),
            }
        }
        pub fn with_src_id(&self, src_id: SrcId) -> StackEntry<'a> {
            let mut entry = self.clone();
            entry.src_id = src_id;
            entry
        }
    }
    struct Stack<'a> {
        stack: Vec<StackEntry<'a>>,
        visited: HashSet<StackEntry<'a>>,
    }
    impl<'a> Stack<'a> {
        pub fn new() -> Self {
            Stack {
                stack: Vec::new(),
                visited: HashSet::new(),
            }
        }
        pub fn push(&mut self, entry: StackEntry<'a>) {
            if self.visited.insert(entry.clone()) {
                self.stack.push(entry);
            }
        }
        pub fn pop(&mut self) -> Option<StackEntry<'a>> {
            self.stack.pop()
        }
    }
    // TODO: Memoize stack hash, to avoid checking same sequence from different combinations
    let mut stack = Stack::new();
    stack.push(StackEntry {
        // (Node, prev_trie_x) represent the trie node to match against
        // Marching through the Trie, we can either:
        // next(node, !prev_x) -> (next(node), node==x)
        // next(node, prev_x)  -> (next(node), node==x)
        //                     -> (node, prev_x)
        node: &trie.root,
        prev_trie_x: false,
        // pattern[idx] is the field that tries to consume the Trie
        pattern_index: 0,
        // Accumulated BitsMatch while walking through the Trie
        bits_match: BitsMatch { mask: 0, value: 0 },
        // Encodings extracted from the Trie
        encoding: Vec::new(),
        // SrcId, last field in encoding
        src_id: SrcId::SrcId,
    });

    while let Some(entry) = stack.pop() {
        if entry.pattern_index == pattern.0.len() && entry.node.is_terminal {
            debug!("Found match with encoding: {:#?}", entry.encoding);
            encodings.push((entry.bits_match, entry.src_id));
            continue;
        } else if entry.pattern_index >= pattern.0.len() || entry.node.is_terminal {
            // Pattern is fully matched but trie path is not terminal,
            // or pattern is not fully matched but trie path is terminal
            continue;
        }

        let pattern_field = &pattern.0[entry.pattern_index];

        let mut check_prev_x = true;

        for (child_field, child_node) in &entry.node.children {
            // FIXME If pattern_field=X and node.children.contains(X), match Xs together and ignore
            // other children
            match (pattern_field, child_field) {
                (Field::SrcId(req_type), trie::Field::SrcId(enc_type)) => {
                    assert!(child_node.is_terminal);
                    let strict_type = match req_type.combine(enc_type) {
                        Some(strict_type) => strict_type,
                        None => {
                            trace!(
                                "Failed to combine SrcId types: ({}, {})",
                                req_type, enc_type
                            );
                            continue;
                        }
                    };

                    let mut new_entry = entry.step_trie(child_field, child_node).step_pattern();
                    if let Some(last) = new_entry.encoding.last_mut() {
                        *last = strict_type.to_string();
                    }
                    stack.push(new_entry.with_src_id(strict_type));
                }
                (Field::Field(name), trie::Field::Named { name: child_name, .. }) => {
                    if name == child_name {
                        stack.push(entry.step_trie(child_field, child_node).step_pattern());
                        check_prev_x = false; // Match implies barrier on the X in the Trie
                    }
                }
                (Field::X, trie::Field::X { .. }) => {
                    stack.push(entry.step_trie(child_field, child_node));
                    stack.push(entry.step_trie(child_field, child_node).step_pattern());
                }
                (Field::X, trie::Field::SrcId(src_id)) => {
                    // End of trie, so advancing only pattern is certainly going to be dropped
                    assert!(child_node.is_terminal);
                    stack.push(
                        entry
                            .step_trie(child_field, child_node)
                            .step_pattern()
                            .with_src_id(src_id.clone()),
                    );
                }
                (Field::Field(_), trie::Field::X { .. }) => {
                    // Trie holds all possible routes that it can be specified,
                    // hence avoid matching specified field in pattern to X in Trie
                }
                (Field::X, trie::Field::Named { .. }) => {
                    // NOTE: Avoid the need to find min products sum, and don't allow Pattern::X to
                    // consume named fields from the Trie
                }
                (Field::Field(_), trie::Field::SrcId(_))     => {}
                (Field::SrcId(_), trie::Field::X { .. })     => {}
                (Field::SrcId(_), trie::Field::Named { .. }) => {}
            }
        }

        if check_prev_x && entry.prev_trie_x {
            // NOTE: Avoid infinite recursion
            // stack.push(entry);
            stack.push(entry.step_pattern());
        }
    }

    if encodings.is_empty() {
        return Err(anyhow::anyhow!(
            "No encodings found matching pattern: {:?}",
            pattern
        ));
    }
    debug!(
        "Found {} encodings matching pattern: {:?}",
        encodings.len(),
        encodings
    );

    // Combine all encodings into one BitsMatch with mask covering all bits and value being the OR of all values
    // FIXME: Replace with combination instead of retruning first match
    let bits_match = encodings
        .iter()
        .fold(BitsMatch { mask: 0, value: 0 }, |acc, enc| {
            acc.combine(&enc.0)
        });

    let src_id = encodings
        .iter()
        .fold(Ok(SrcId::SrcId) as Result<SrcId>, |acc, enc| {
            let acc = acc?;
            match acc.combine(&enc.1) {
                Some(strict_type) => Ok(strict_type),
                None => Err(anyhow::anyhow!(
                    "Failed to combine SrcId types: ({}, {})",
                    acc,
                    enc.1
                )),
            }
        })?;

    Ok((bits_match, src_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SrcId;
    use crate::Trie;
    use et_core::bits::BitsMatch;
    use crate::trie::{Encoding, Field as TrieField};

    fn make_trie_named(name: &str, mask: u32, value: u32) -> TrieField {
        TrieField::Named {
            name: name.to_string(),
            attr: None,
            bits: BitsMatch { mask, value },
            size: 4,
        }
    }

    fn make_trie_srcid(src_id: SrcId) -> TrieField {
        TrieField::SrcId(src_id)
    }

    fn make_trie_x(size: usize) -> TrieField {
        TrieField::X { attr: None, size }
    }

    #[test]
    fn pattern_new() {
        let pattern = Pattern(vec![Field::Field("MCRT"), Field::SrcId(SrcId::MatSurfId)]);
        assert_eq!(pattern.0.len(), 2);
    }

    #[test]
    fn search_trie_simple_match() {
        let mut trie = Trie::new();
        trie.insert(&Encoding(vec![
            make_trie_named("MCRT", 0xF0000000, 0x30000000),
            make_trie_srcid(SrcId::MatSurfId),
        ]));

        let pattern = Pattern(vec![Field::Field("MCRT"), Field::SrcId(SrcId::MatSurfId)]);

        let result = trie.get(&pattern);
        assert!(result.is_ok());
    }

    #[test]
    fn search_trie_no_match() {
        let mut trie = Trie::new();
        trie.insert(&Encoding(vec![
            make_trie_named("MCRT", 0xF0000000, 0x30000000),
            make_trie_srcid(SrcId::MatSurfId),
        ]));

        let pattern = Pattern(vec![Field::Field("Emission"), Field::SrcId(SrcId::LightId)]);

        let result = trie.get(&pattern);
        assert!(result.is_err());
    }

    #[test]
    fn search_trie_x_consumes() {
        let mut trie = Trie::new();
        trie.insert(&Encoding(vec![
            make_trie_named("MCRT", 0xF0000000, 0x30000000),
            make_trie_x(4),
            make_trie_srcid(SrcId::MatId),
        ]));

        let pattern = Pattern(vec![
            Field::Field("MCRT"),
            Field::X,
            Field::SrcId(SrcId::MatId),
        ]);

        let result = trie.get(&pattern);
        assert!(result.is_ok());
    }

    #[test]
    fn search_trie_srcid_type_combine() {
        let mut trie = Trie::new();
        trie.insert(&Encoding(vec![
            make_trie_named("MCRT", 0xF0000000, 0x30000000),
            make_trie_srcid(SrcId::MatSurfId),
        ]));

        let pattern = Pattern(vec![Field::Field("MCRT"), Field::SrcId(SrcId::SrcId)]);

        let result = trie.get(&pattern);
        assert!(result.is_ok());
        let (_bits, src_id) = result.unwrap();
        assert_eq!(src_id, SrcId::MatSurfId);
    }
}
