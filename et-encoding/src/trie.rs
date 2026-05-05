//! This module defines the Trie data structure used to store and query encoding patterns.
//! The Trie allows for efficient querying of encoding patterns based on a given Pattern, which can contain named fields, SrcId fields, and don't care fields (X).
//! The Trie also supports emitting a DOT string for visualization using graphviz.

use crate::{
    SrcId,
    pattern::{Pattern, search_trie},
};
use et_core::bits::BitsMatch;
use anyhow::{Error, Result, anyhow};
use std::{collections::HashSet, str::FromStr};

/// Field similar to [`crate::pattern::Field`], explicit definition from the encoding spec
#[allow(clippy::derived_hash_with_manual_eq)]
#[derive(Debug, Clone, Hash)]
pub(crate) enum Field {
    /// Don't care, with attribute to hot swap all valid values with the same attribute
    /// i.e. `X {Direction}` might take the value from `Forward {Direction}`, etc. from another
    /// entry in the Trie. This enables compact specification, but ability to fully expand the
    /// Trie to all possible encodings.
    X {
        attr: Option<String>,
        size: usize,
    },
    /// SrcId type
    SrcId(SrcId),
    /// Named field with a specific value (or __partial__ value) and optional attribute that can
    /// be used to hot swap in X or identify SrcId type
    Named {
        name: String,
        attr: Option<String>,
        bits: BitsMatch,
        size: usize,
    },
}

impl Field {
    /// Returns the BitsMatch condition for this field
    pub fn bits_match(&self) -> &BitsMatch {
        match self {
            Field::X { .. }           => &BitsMatch { mask: 0, value: 0 },
            Field::SrcId(_)           => &BitsMatch { mask: 0, value: 0 },
            Field::Named { bits, .. } => bits,
        }
    }
    /// Bits size of this field
    pub fn size(&self) -> usize {
        match self {
            Field::X { size, .. }     => *size,
            Field::SrcId(_)           => 16,
            Field::Named { size, .. } => *size,
        }
    }
}

impl FromStr for Field {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Ok(src_id) = SrcId::from_str(s) {
            Ok(Field::SrcId(src_id))
        } else {
            Err(anyhow!("Unknown field type: {}", s))
        }
    }
}

impl std::fmt::Display for Field {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Field::X{size, attr: Some(attr)}            => write!(f, "X({size} bits)\n{{{attr}}}"),
            Field::X{size, ..}                          => write!(f, "X({size} bits)"),
            Field::SrcId(src_id)                        => write!(f, "{src_id}"),
            Field::Named { name, attr: Some(attr), .. } => write!(f, "{attr}::{name}"),
            Field::Named { name, .. }                   => write!(f, "{}", name),
        }
    }
}

impl PartialEq for Field {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Field::X { size: bits1, .. }, Field::X { size: bits2, .. }) => bits1 == bits2,
            (
                Field::Named {
                    name: name1,
                    attr: _,
                    bits: bits1,
                    size: size1,
                },
                Field::Named {
                    name: name2,
                    attr: _,
                    bits: bits2,
                    size: size2,
                },
            ) => {
                // name1==name2 |-> bits1==bits2 and size1==size2
                debug_assert!(
                    name1 != name2 || (bits1 == bits2 && size1 == size2),
                    "Field with the same name has different encodings"
                );
                name1 == name2
            }
            (Field::SrcId(src_id1), Field::SrcId(src_id2)) => src_id1 == src_id2,
            _ => false,
        }
    }
}
impl Eq for Field {}

/// Encoding represents a complete encoding pattern, which is a sequence of fields that together define full or partial 32-bit encoding. By partial, we mean that some fields could be considered don't care.
#[derive(Debug)]
pub struct Encoding(pub(crate) Vec<Field>);

/// This macro provides a convenient way to construct an Encoding from a list of field names and their corresponding bit patterns, along with a final SrcId field.
#[macro_export]
macro_rules! encoding {
    ($($field:ident, $bits:expr),* ,$src_id:ident) => {
        let lens = vec![$($bits),*].iter().map(|b| stringify!($src_id).split("0b")[1].len()).sum::<usize>();
        Encoding(vec![
            $(Field::Named(stringify!($field).to_string(), BitsMatch::parse(&$bits))),* ,
             Field::Named(stringify!($src_id).to_string(), BitsMatch::parse(&BitsRange(15, 0)))
        ])
    };
}

/// TrieNode represents a node in the trie, which can be either a terminal node (representing a complete encoding) or an internal node with children representing possible next fields in the encoding pattern.
// FIXME: no need for public fields
#[derive(Debug, Hash, PartialEq, Eq)]
pub(crate) struct TrieNode {
    pub(crate) is_terminal: bool,
    pub(crate) children: Vec<(Field, TrieNode)>,
}

/// Trie represents the root of the trie data structure
#[derive(Debug)]
pub struct Trie {
    pub(crate) root: TrieNode,
}

impl Default for Trie {
    fn default() -> Self {
        Self::new()
    }
}

impl Trie {
    /// Returns a new, empty Trie.
    pub fn new() -> Self {
        Self {
            root: TrieNode::new(),
        }
    }

    /// Insert an encoding into the trie
    ///
    /// # Examples
    ///
    /// ```ignored
    /// use et_encoding::trie::{Trie, Encoding};
    /// use et_encoding::trie::Field;
    /// use et_encoding::bits::BitsMatch;
    /// use et_encoding::SrcId;
    /// use et_encoding::encoding;
    ///
    /// let mut trie = Trie::new();
    /// let encoding = encoding!(MCRT, 0b0011, Material, 0b01, Elastic, 0b10, X, 0b0000, MatSurfId);
    ///
    /// trie.insert(encoding);
    /// assert_eq!(vec![BitsMatch{mask: 0x0FA00000, value: 0x0360000}], trie.get_all());
    /// ```
    pub fn insert(&mut self, encoding: &Encoding) {
        let mut current = &mut self.root;

        for field in &encoding.0 {
            if current.get_mut(field).is_none() {
                current.insert_new(field);
            }

            current = current.get_mut(field).unwrap();
        }

        current.is_terminal = true;
    }

    /// Emits a DOT format string representing the trie, which can be visualized using Graphviz.
    pub fn emit_dot(&self) -> String {
        fn walk(node: &TrieNode, id: usize, next_id: &mut usize, out: &mut String) {
            if node.is_terminal {
                out.push_str(&format!("n{id} [label=\"{}\", shape=doublecircle];\n", id));
            } else {
                out.push_str(&format!("n{id} [label=\"{}\", shape=circle];\n", id));
            }

            for (field, child) in &node.children {
                *next_id += 1;
                let child_id = *next_id;
                out.push_str(&format!("n{id} -> n{child_id} [label=\"{}\"];\n", field));
                walk(child, child_id, next_id, out);
            }
        }

        let mut out = String::from("digraph Trie {\nrankdir=TB;\nnode [fontname=\"Arial\"];\n");
        let mut next_id = 0;
        walk(&self.root, 0, &mut next_id, &mut out);
        out.push_str("}\n");
        out
    }

    /// Returns the bits match and SrcId field type for the specified Pattern
    ///
    /// # Examples
    ///
    /// ```ignored
    /// use et_encoding::trie::{Trie, Encoding};
    /// use et_encoding::trie::Field;
    /// use et_encoding::pattern::{Pattern, self};
    /// use et_encoding::bits::BitsMatch;
    /// use et_encoding::SrcId;
    /// use et_encoding::encoding;
    ///
    /// let mut trie = Trie::new();
    /// let encoding = encoding!(MCRT, 0b0011, Material, 0b01, Elastic, 0b10, X, 0b0000, MatSurfId);
    ///
    /// trie.insert(encoding);
    /// let pattern = Pattern(vec![
    ///     pattern::Field::Field("MCRT"),
    ///     pattern::Field::Field("Material"),
    ///     pattern::Field::Field("Elastic"),
    ///     pattern::Field::X,
    ///     pattern::Field::SrcId(SrcId::MatId),
    /// ]);
    /// let (bits_match, src_id_type) = trie.get(&pattern).unwrap();
    /// assert_eq!(bits_match, BitsMatch{mask: 0x0ff00000, value: 0x03600000});
    /// ```
    pub fn get(&self, query: &Pattern) -> Result<(BitsMatch, SrcId)> {
        search_trie(self, query)
    }

    /// Returns a list of all fields
    pub fn get_fields(&self) -> HashSet<String> {
        let mut fields = HashSet::new();
        let mut stack = vec![&self.root];
        while let Some(node) = stack.pop() {
            for (field, child) in &node.children {
                if !child.is_terminal {
                    if let Field::Named { name, .. } = field {
                        fields.insert(name.to_string());
                    }
                    stack.push(child);
                }
            }
        }
        fields
    }
}

impl TrieNode {
    /// Returns a new TrieNode with no children and is_end set to false.
    pub fn new() -> Self {
        Self {
            is_terminal: false,
            children: Vec::new(),
        }
    }

    /// Returns a mutable reference to the child node corresponding to the given field, if it exists.
    pub(crate) fn get_mut(&mut self, field: &Field) -> Option<&mut TrieNode> {
        self.children
            .iter_mut()
            .find_map(|(f, node)| if f == field { Some(node) } else { None })
    }

    /// Inserts a new child node for the given field, if it does not already exist.
    pub(crate) fn insert_new(&mut self, field: &Field) {
        self.children.push((field.clone(), TrieNode::new()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SrcId;
    use et_core::bits::BitsMatch;

    fn make_named_field(name: &str, mask: u32, value: u32, size: usize) -> Field {
        Field::Named {
            name: name.to_string(),
            attr: None,
            bits: BitsMatch { mask, value },
            size,
        }
    }

    fn make_srcid_field(src_id: SrcId) -> Field {
        Field::SrcId(src_id)
    }

    fn make_x_field(size: usize) -> Field {
        Field::X { attr: None, size }
    }

    #[test]
    fn trie_node_new() {
        let node = TrieNode::new();
        assert!(!node.is_terminal);
        assert!(node.children.is_empty());
    }

    #[test]
    fn trie_new() {
        let trie = Trie::new();
        assert!(!trie.root.is_terminal);
        assert!(trie.root.children.is_empty());
    }

    #[test]
    fn trie_insert_single_encoding() {
        let mut trie = Trie::new();
        let encoding = Encoding(vec![
            make_named_field("MCRT", 0xF0000000, 0x30000000, 4),
            make_srcid_field(SrcId::MatSurfId),
        ]);
        trie.insert(&encoding);
        assert!(!trie.root.is_terminal);
        assert_eq!(trie.root.children.len(), 1);
    }

    #[test]
    fn trie_insert_multiple_encodings() {
        let mut trie = Trie::new();
        trie.insert(&Encoding(vec![
            make_named_field("MCRT", 0xF0000000, 0x30000000, 4),
            make_srcid_field(SrcId::MatSurfId),
        ]));
        trie.insert(&Encoding(vec![
            make_named_field("Emission", 0xF0000000, 0x10000000, 4),
            make_srcid_field(SrcId::LightId),
        ]));
        assert_eq!(trie.root.children.len(), 2);
    }

    #[test]
    fn trie_insert_shared_prefix() {
        let mut trie = Trie::new();
        trie.insert(&Encoding(vec![
            make_named_field("MCRT", 0xF0000000, 0x30000000, 4),
            make_named_field("Material", 0x0C000000, 0x08000000, 2),
            make_srcid_field(SrcId::MatId),
        ]));
        trie.insert(&Encoding(vec![
            make_named_field("MCRT", 0xF0000000, 0x30000000, 4),
            make_named_field("Interface", 0x0C000000, 0x00000000, 2),
            make_srcid_field(SrcId::MatSurfId),
        ]));
        assert_eq!(trie.root.children.len(), 1);
        let mcrt_children = &trie.root.children[0].1.children;
        assert_eq!(mcrt_children.len(), 2);
    }

    #[test]
    fn trie_insert_terminal_node() {
        let mut trie = Trie::new();
        trie.insert(&Encoding(vec![
            make_named_field("MCRT", 0xF0000000, 0x30000000, 4),
            make_srcid_field(SrcId::MatSurfId),
        ]));
        let mcrt_child = trie
            .root
            .get_mut(&make_named_field("MCRT", 0xF0000000, 0x30000000, 4))
            .unwrap();
        let srcid_child = mcrt_child
            .get_mut(&make_srcid_field(SrcId::MatSurfId))
            .unwrap();
        assert!(srcid_child.is_terminal);
    }

    #[test]
    fn trie_get_mut_existing() {
        let mut trie = Trie::new();
        let field = make_named_field("MCRT", 0xF0000000, 0x30000000, 4);
        let encoding = Encoding(vec![field.clone(), make_srcid_field(SrcId::MatSurfId)]);
        trie.insert(&encoding);

        let node = trie.root.get_mut(&field);
        assert!(node.is_some());
    }

    #[test]
    fn trie_get_mut_nonexistent() {
        let mut trie = Trie::new();
        let field = make_named_field("MCRT", 0xF0000000, 0x30000000, 4);
        let node = trie.root.get_mut(&field);
        assert!(node.is_none());
    }

    #[test]
    fn trie_emit_dot_format() {
        let mut trie = Trie::new();
        trie.insert(&Encoding(vec![
            make_named_field("MCRT", 0xF0000000, 0x30000000, 4),
            make_srcid_field(SrcId::MatSurfId),
        ]));

        let dot = trie.emit_dot();
        assert!(dot.contains("digraph Trie"));
        assert!(dot.contains("n0"));
        assert!(dot.contains("->"));
    }

    #[test]
    fn field_size() {
        let x_field = make_x_field(8);
        assert_eq!(x_field.size(), 8);

        let named = make_named_field("MCRT", 0xF0000000, 0x30000000, 4);
        assert_eq!(named.size(), 4);

        let srcid = make_srcid_field(SrcId::MatSurfId);
        assert_eq!(srcid.size(), 16);
    }

    #[test]
    fn field_bits_match() {
        let x_field = make_x_field(8);
        assert_eq!(x_field.bits_match(), &BitsMatch { mask: 0, value: 0 });

        let named = make_named_field("MCRT", 0xF0000000, 0x30000000, 4);
        assert_eq!(named.bits_match(), &BitsMatch {
            mask: 0xF0000000,
            value: 0x30000000
        });

        let srcid = make_srcid_field(SrcId::MatSurfId);
        assert_eq!(srcid.bits_match(), &BitsMatch { mask: 0, value: 0 });
    }

    #[test]
    fn field_partial_eq() {
        let f1 = make_named_field("MCRT", 0xF0000000, 0x30000000, 4);
        let f2 = make_named_field("MCRT", 0xF0000000, 0x30000000, 4);
        let f3 = make_named_field("Interface", 0xF0000000, 0x30000000, 4);
        assert_eq!(f1, f2);
        assert_ne!(f1, f3);

        let x1 = make_x_field(8);
        let x2 = make_x_field(8);
        let x3 = make_x_field(4);
        assert_eq!(x1, x2);
        assert_ne!(x1, x3);

        let s1 = make_srcid_field(SrcId::MatSurfId);
        let s2 = make_srcid_field(SrcId::MatSurfId);
        let s3 = make_srcid_field(SrcId::LightId);
        assert_eq!(s1, s2);
        assert_ne!(s1, s3);
    }

    #[test]
    fn field_display() {
        let named = make_named_field("MCRT", 0xF0000000, 0x30000000, 4);
        assert_eq!(format!("{}", named), "MCRT");

        let srcid = make_srcid_field(SrcId::MatSurfId);
        assert_eq!(format!("{}", srcid), "SrcId::MatSurfId");

        let x_field = make_x_field(8);
        assert_eq!(format!("{}", x_field), "X(8 bits)");
    }

    #[test]
    fn encoding_new() {
        let encoding = Encoding(vec![
            make_named_field("MCRT", 0xF0000000, 0x30000000, 4),
            make_srcid_field(SrcId::MatSurfId),
        ]);
        assert_eq!(encoding.0.len(), 2);
    }

    #[test]
    fn trie_get_fields() {
        let mut trie = Trie::new();
        trie.insert(&Encoding(vec![
            make_named_field("MCRT", 0xF0000000, 0x30000000, 4),
            make_named_field("Material", 0x0C000000, 0x08000000, 2),
            make_srcid_field(SrcId::MatId),
        ]));

        let fields = trie.get_fields();
        assert!(fields.contains("MCRT"));
        assert!(fields.contains("Material"));
        assert!(!fields.contains("Interface"));
    }
}
