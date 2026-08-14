//! Model - Semantic model for filter DSL
//!
//! This module provides the semantic model that represents
//! the meaning of parsed filter DSL declarations.
//!
//! The model converts AST expressions to concrete matching structures
//! that can efficiently evaluate events.

use std::collections::HashMap;

use events_ledger::{SrcId as DomainSrcId, ledger::SrcName};
use et_core::{Repetition, bits::BitsMatch};
use et_encoding::{
    pattern::{self, Pattern},
    trie::Trie,
};
use log::debug;

use crate::Check;
use crate::{
    ast::{DeclType, Declaration, Expr, SrcId},
    error::Error,
    failure,
};

// -------------------------------------------------
// AST -> Semantics Model
// -------------------------------------------------

/// Represents a matching pattern for event evaluation.
///
/// This is the runtime representation of a pattern that can be
/// checked against event data to determine if there's a match.
#[derive(Debug, Clone, PartialEq)]
pub enum Match {
    /// Wildcard (matches anything)
    X,
    /// Bitwise pattern matching
    Bits(BitsMatch),
    /// Negation of a pattern
    Not(Box<Match>),
    /// Conjunction (both must match)
    And(Box<Match>, Box<Match>),
    /// Disjunction (any must match)
    Any(Vec<Match>),
}

impl Match {
    /// Optimise the match by applying simplification rules.
    pub fn optimise(self) -> Self {
        match self {
            Match::And(ref left, ref right) => match (left.as_ref(), right.as_ref()) {
                (Match::Bits(bits1), Match::Bits(bits2)) => Match::Bits(bits1.combine(bits2)),
                (Match::X, other) | (other, Match::X) => other.clone().optimise(),
                _ => self,
            },
            Match::Any(matches) => {
                assert!(
                    !matches.is_empty(),
                    "Any match should have at least one inner match"
                );

                let mut flattened = Vec::new();
                for m in matches.into_iter() {
                    match m.optimise() {
                        Match::Any(inner) => flattened.extend(inner),
                        other => flattened.push(other),
                    }
                }

                if flattened.len() == 1 {
                    flattened[0].to_owned()
                } else {
                    for m in &flattened {
                        if matches!(m, Match::X) {
                            return Match::X;
                        }
                    }
                    // TODO: Optimise logic operations found for x86
                    // to evaluate an u32 value with Match::Any
                    //   - SAT solver or some for of boolean expression optimisation
                    //   - Construct tree with minimum depth that pairs overlapping masks
                    Match::Any(flattened)
                }
            }
            other => other,
        }
    }
}

impl From<BitsMatch> for Match {
    fn from(bits: BitsMatch) -> Self {
        Match::Bits(bits)
    }
}

impl Check<u32> for Match {
    fn check(&self, event_encoded: u32) -> bool {
        match self {
            Match::X => true,
            Match::Bits(bits_match) => bits_match.check(event_encoded),
            Match::Not(bits_match) => !bits_match.check(event_encoded),
            Match::And(left, right) => left.check(event_encoded) && right.check(event_encoded),
            Match::Any(matches) => matches.iter().any(|m| m.check(event_encoded)),
        }
    }
}

impl From<SrcId<'_>> for et_encoding::SrcId {
    fn from(other: SrcId) -> Self {
        match other {
            SrcId::None            => et_encoding::SrcId::SrcId,
            SrcId::Mat(_)          => et_encoding::SrcId::MatId,
            SrcId::Surf(_)         => et_encoding::SrcId::SurfId,
            SrcId::MatSurf(_)      => et_encoding::SrcId::MatSurfId,
            SrcId::Light(_)        => et_encoding::SrcId::LightId,
            SrcId::Detector(_)     => et_encoding::SrcId::DetectorId,
            SrcId::MatName(_)      => et_encoding::SrcId::MatId,
            SrcId::SurfName(_)     => et_encoding::SrcId::SurfId,
            SrcId::MatSurfName(_)  => et_encoding::SrcId::MatSurfId,
            SrcId::LightName(_)    => et_encoding::SrcId::LightId,
            SrcId::DetectorName(_) => et_encoding::SrcId::DetectorId,
        }
    }
}

/// A predicate modifier for pattern matching.
///
/// Applies additional constraints to patterns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Predicate {
    /// Unit=None - No additional predicate, just match the pattern as is
    Unit,
    /// Not - Negate the pattern match ("! pattern")
    Not,
    /// Repeat - Match a pattern with specified repetition ("{m,n} pattern")
    Repeat(Repetition),
}

/// Sequence Tree
/// Represents a sequence of patterns to match in order.
///
///  - Used for rules that require specific ordering of events.
#[derive(Debug, Clone)]
pub enum SeqTree {
    /// A pattern with optional predicate
    Pattern(Predicate, Match),
    /// Permutation - any order
    Perm(Vec<Self>),
    /// Sequence - ordered list
    Seq(Vec<Self>),
}

impl SeqTree {
    /// Flatten nested Seq
    pub fn optimise(self) -> Self {
        match self {
            SeqTree::Seq(seq) => {
                let mut flattened = Vec::new();
                for s in seq.into_iter() {
                    match s.optimise() {
                        SeqTree::Seq(inner) => flattened.extend(inner),
                        other => flattened.push(other),
                    }
                }
                if flattened.len() == 1 {
                    flattened[0].to_owned()
                } else {
                    SeqTree::Seq(flattened)
                }
            }
            other => other,
        }
    }

    /// Lower the SeqTree into a flat sequence of predicate-pattern pairs.
    pub fn lower(self) -> Seq {
        match self {
            SeqTree::Pattern(pred, pattern_match) => Seq(vec![(pred, pattern_match)]),
            SeqTree::Seq(seq) => {
                let mut lowered = Vec::new();
                for s in seq.into_iter() {
                    lowered.extend(s.lower().0);
                }
                Seq(lowered)
            }
            SeqTree::Perm(_seq) => todo!("Permutation feature not implemented yet"),
        }
    }
}

/// A sequence of predicate-match pairs.
///
/// This represents a flattened sequence for efficient matching.
#[derive(Debug)]
pub struct Seq(pub Vec<(Predicate, Match)>);

/// A condition in a rule.
///
/// Rules can have pattern conditions or sequence conditions.
#[derive(Debug)]
pub enum RuleCond {
    /// Match a specific pattern
    Pattern(Predicate, Match),
    /// Match a sequence
    Seq(Seq),
}

/// A rule definition.
///
/// A rule contains a list of conditions that must be satisfied.
#[derive(Debug)]
pub struct Rule(pub Vec<RuleCond>);

// -------------------------------------------------
// Processing AST into Semantics Model
// -------------------------------------------------

/// Resolve AST declarations into a semantic model.
///
/// Converts parsed AST declarations into a map of rule names
/// to their resolved semantic representations.
///
/// # Arguments
///
/// * `src` - Source code for error reporting
/// * `declarations` - Parsed AST declarations
/// * `src_dict` - Mapping of source names to IDs
/// * `trie` - Pattern trie for field lookup
///
/// # Returns
///
/// A map from rule names to their resolved [`Rule`] representations
pub fn resolve_ast(
    src: &str,
    declarations: &Vec<Declaration>,
    src_dict: &HashMap<SrcName, DomainSrcId>,
    trie: &Trie,
) -> HashMap<String, Rule> {
    let mut resolved_src_dict = HashMap::new();
    for decl in declarations.iter() {
        let (expr, span) = &decl.body;
        if decl.decl_type == DeclType::SrcId {
            let resolved_src_id = expr
                .resolve_src(src_dict)
                .unwrap_or_else(|e| {
                    let err = e.with_span(*span);
                    failure(
                        format!("Failed to resolve source identifier {:?}: {}", expr, err),
                        ("not found inside Ledger".into(), err.span().unwrap()),
                        None,
                        src,
                    )
                });

            debug!("Resolved {:?} into: {:?}", expr, resolved_src_id);
            resolved_src_dict.insert(decl.name, resolved_src_id);
        }
    }

    let mut resolved_pattern_dict = HashMap::new();
    for decl in declarations.iter() {
        let (expr, span) = &decl.body;
        if decl.decl_type == DeclType::Pattern {
            let resolved_pattern = expr
                .resolve_pattern(src_dict, trie, &resolved_src_dict)
                .unwrap_or_else(|e| {
                    let err = e.with_span(*span);
                    failure(
                        format!("Failed to resolve pattern {:?}: {}", expr, err),
                        ("not found inside Trie".into(), err.span().unwrap()),
                        None,
                        src,
                    );
                });
            debug!("Resolved {:?} into: {:?}", expr, resolved_pattern);
            resolved_pattern_dict.insert(decl.name, resolved_pattern);
        }
    }

    let mut resolved_seq_dict = HashMap::new();
    for decl in declarations.iter() {
        let (expr, span) = &decl.body;
        if decl.decl_type == DeclType::Sequence {
            let resolved_seq = expr
                .resolve_seq(src_dict, trie, &resolved_src_dict, &resolved_pattern_dict)
                .unwrap_or_else(|e| {
                    let err = e.with_span(*span);
                    failure(
                        format!("Failed to resolve sequence {:?}: {}", expr, err),
                        ("failed to resolve sequence".into(), err.span().unwrap()),
                        None,
                        src,
                    );
                });
            debug!("Resolved {:?} into: {:?}", expr, resolved_seq);
            resolved_seq_dict.insert(decl.name, resolved_seq);
        }
    }

    let mut resolved_rule_dict = HashMap::new();
    for decl in declarations.iter() {
        let (expr, span) = &decl.body;
        if decl.decl_type == DeclType::Rule {
            let resolved_rule = expr
                .resolve_rule(
                    src_dict,
                    trie,
                    &resolved_src_dict,
                    &resolved_pattern_dict,
                    &resolved_seq_dict,
                )
                .unwrap_or_else(|e| {
                    let err = e.with_span(*span);
                    failure(
                        format!("Failed to resolve rule {:?}: {}", expr, err),
                        ("failed to resolve rule".into(), err.span().unwrap()),
                        None,
                        src,
                    );
                });
            debug!("Resolved {:?} into: {:?}", expr, resolved_rule);
            resolved_rule_dict.insert(decl.name.to_owned(), resolved_rule);
        }
    }

    resolved_rule_dict
}

impl<'src> Expr<'src> {
    pub fn resolve_src(
        &self,
        src_dict: &HashMap<SrcName, DomainSrcId>,
    ) -> Result<(et_encoding::SrcId, Match), Error> {
        Ok(match self {
            Self::X => (et_encoding::SrcId::SrcId, Match::X),
            Self::SrcId(src_id) => {
                debug!("Resolving source identifier: {:?}", src_id);
                let src_bits_match = src_id
                        .resolve(src_dict)?
                        .bits_match()
                        .into();
                (src_id.clone().into(), Match::Bits(src_bits_match))
            }
            // TODO: If error occur extract the span and propagate it back as Result
            // Perhaps make use of ariadne to properly show diagnostics relevant to code
            Self::Any(exprs) => {
                let src_id_matches: Vec<_> = exprs
                    .iter()
                    .map(|(expr, span)| {
                        expr.resolve_src(src_dict)
                            .map_err(|err| err.with_span(*span))
                    })
                    .collect::<Result<_, _>>()?;
                let src_id_type = src_id_matches
                    .iter()
                    .fold(et_encoding::SrcId::SrcId, |acc, (src_id, _)| {
                        acc.combine(src_id).unwrap()
                    });
                let src_id_match = Match::Any(
                    src_id_matches
                        .into_iter()
                        .map(|(_, r#match)| r#match)
                        .collect(),
                );
                debug!(
                    "Combined any SrcId types into: ({:?}, {:?})",
                    src_id_type, src_id_match
                );
                // TODO: flatten nested Any
                (src_id_type, src_id_match.optimise())
            }
            _ => return Err(Error::Unspanned(format!("Unexpected expression type for resolving SrcId: {:?}", self))),
        }
        )
    }

    pub fn resolve_pattern(
        &self,
        src_dict: &HashMap<SrcName, DomainSrcId>,
        trie: &Trie,
        resolved_src: &HashMap<&'src str, (et_encoding::SrcId, Match)>,
    ) -> Result<Match, Error> {
        Ok(match self {
            Self::X => Match::X,
            Self::Pattern(fields) => {
                let src_field = match &fields.last() {
                    Some(field) => &field.0,
                    // TODO: Include span information
                    None => {
                        return Err(Error::Unspanned(
                            "Pattern must have at least one field (the SrcId)".into(),
                        ));
                    }
                };
                let (src_id_type, src_id_match) = match src_field {
                    Expr::SrcId(_) | Expr::Any(_) => src_field.resolve_src(src_dict)?,
                    Expr::Ident(src_ident) => {
                        match resolved_src.get(*src_ident) {
                            Some(ident) => ident.clone(),
                            None => return Err(Error::Unspanned(format!("Unknown source identifier in pattern: {}", src_ident))),
                        }
                    }
                    _ => return Err(Error::Unspanned(format!("Unexpected expression type for SrcId in pattern: {:?}", src_field))),
                };

                let mut fields: Vec<_> = fields[..fields.len() - 1]
                    .iter()
                    .map(|(field, span)| match field {
                        Expr::X => Ok(pattern::Field::X),
                        Expr::Field(field_id) => Ok(pattern::Field::Field(field_id)),
                        _ => Err(Error::Spanned {
                            msg: format!("Unexpected expression type in pattern fields: {:?}", field),
                            span: *span,
                        }),
                    })
                    .collect::<Result<_,_>>()?;

                fields.push(pattern::Field::SrcId(src_id_type));
                let pattern = Pattern(fields);
                let (bits_match, _src_id_type) = trie
                    .get(&pattern)
                    .map_err(|err|
                        Error::Unspanned(format!("{}", err.context("Failed to get pattern from trie")))
                    )?;
                Match::And(Box::new(Match::Bits(bits_match)), Box::new(src_id_match)).optimise()
            }
            Self::Any(exprs) => {
                let bits_match = exprs
                    .iter()
                    .map(|(e, span)| e
                        .resolve_pattern(src_dict, trie, resolved_src)
                        .map_err(|err| err.with_span(*span))
                    )
                    .collect::<Result<_, _>>()?;
                Match::Any(bits_match).optimise()
            }
            _ => {
                return Err(Error::Unspanned(format!(
                    "Unexpected expression type for resolving SrcId: {:?}",
                    self
                )));
            }
        })
    }

    pub fn resolve_seq(
        &self,
        src_dict: &HashMap<SrcName, DomainSrcId>,
        trie: &Trie,
        resolved_src: &HashMap<&'src str, (et_encoding::SrcId, Match)>,
        resolved_pattern: &HashMap<&'src str, Match>,
    ) -> Result<SeqTree, Error> {
        Ok(match self {
            Self::X => SeqTree::Pattern(Predicate::Unit, Match::X),
            Self::Seq(exprs) => SeqTree::Seq(
                exprs
                    .iter()
                    .map(|(e, span)| e
                        .resolve_seq(src_dict, trie, resolved_src, resolved_pattern)
                        .map_err(|err| err.with_span(*span))
                    )
                    .collect::<Result<_,_>>()?
            ),
            Self::Perm(_exprs) => return Err(Error::Unspanned("Permutation feature not implemented yet".to_string())),
            Self::Any(exprs) => SeqTree::Pattern(
                Predicate::Unit,
                Match::Any( exprs
                    .iter()
                    .map(|(e, span)| e
                        .resolve_pattern(src_dict, trie, resolved_src)
                        .map_err(|err| err.with_span(*span))
                    )
                    .collect::<Result<_,_>>()?
                ).optimise()
            ),
            Self::Repeat(repetition, boxed) => {
                let (pattern, span) = boxed.as_ref();
                let pred_pattern_match = pattern
                    .resolve_seq(src_dict, trie, resolved_src, resolved_pattern)
                    .map_err(|err| err.with_span(*span))?;
                if let SeqTree::Pattern(Predicate::Unit, pattern_match) = pred_pattern_match {
                    SeqTree::Pattern(Predicate::Repeat(repetition.clone()), pattern_match)
                } else {
                    return Err(Error::Unspanned(format!(
                        "Expected a pattern in sequence repetition, found: {:?}",
                        pattern
                    ))
                    .with_span(*span));
                }
            }
            // FIXME: Ident could also be a sequence, which results in a nested seq to be flatten
            Self::Ident(pattern_name) => match resolved_pattern.get(*pattern_name) {
                Some(pattern_match) => SeqTree::Pattern(Predicate::Unit, pattern_match.clone()),
                None => {
                    return Err(Error::Unspanned(format!(
                        "Unknown pattern in sequence: {}",
                        pattern_name
                    )));
                }
            },
            Self::Pattern(_expr) => {
                let pattern_match = self.resolve_pattern(src_dict, trie, resolved_src)?;
                SeqTree::Pattern(Predicate::Unit, pattern_match)
            }
            Self::Not(boxed) => {
                let (pattern, span) = boxed.as_ref();
                let pred_pattern_match = pattern
                    .resolve_seq(src_dict, trie, resolved_src, resolved_pattern)
                    .map_err(|err| err.with_span(*span))?;
                if let SeqTree::Pattern(Predicate::Unit, pattern_match) = pred_pattern_match {
                    SeqTree::Pattern(Predicate::Not, pattern_match)
                } else {
                    return Err(Error::Unspanned(
                        format!("Expected a pattern in negation, found: {:?}", pattern)
                    ).with_span(*span));
                }
            }
            _ => {
                return Err(Error::Unspanned(format!(
                    "Unexpected expression type for resolving sequence: {:?}",
                    self
                )));
            }
        })
    }

    pub fn resolve_rule_cond(
        &self,
        src_dict: &HashMap<SrcName, DomainSrcId>,
        trie: &Trie,
        resolved_src: &HashMap<&'src str, (et_encoding::SrcId, Match)>,
        resolved_pattern: &HashMap<&'src str, Match>,
        resolved_seq: &HashMap<&'src str, SeqTree>,
    ) -> Result<RuleCond, Error> {
        Ok(match self {
            Self::Pattern(_) => RuleCond::Pattern(
                Predicate::Unit,
                self.resolve_pattern(src_dict, trie, resolved_src)?,
            ),
            Self::Repeat(repetition, boxed) => {
                let (pattern, span) = boxed.as_ref();
                // TODO: Also support Ident
                let pred_pattern_match = pattern
                    .resolve_rule_cond(src_dict, trie, resolved_src, resolved_pattern, resolved_seq)
                    .map_err(|err| err.with_span(*span))?;
                if let RuleCond::Pattern(Predicate::Unit, pattern_match) = pred_pattern_match {
                    RuleCond::Pattern(Predicate::Repeat(repetition.clone()), pattern_match)
                } else {
                    return Err(Error::Unspanned(format!(
                        "Expected a pattern in repetition, found: {:?}",
                        pattern
                    ))
                    .with_span(*span));
                }
            }
            Self::Any(exprs) => RuleCond::Pattern(
                Predicate::Unit,
                Match::Any(exprs
                    .iter()
                    .map(|(expr, span)| {
                        let inner_rule_cond = expr
                            .resolve_rule_cond(src_dict, trie, resolved_src, resolved_pattern, resolved_seq)
                            .map_err(|err| err.with_span(*span))?;
                        if let RuleCond::Pattern(Predicate::Unit, pattern_match) = inner_rule_cond {
                            Ok(pattern_match)
                        } else {
                            Err(Error::Unspanned(
                                format!("Unexpected expression type in condition Any: {:?}", expr)
                            ).with_span(*span))
                        }
                    })
                    .collect::<Result<_,_>>()?,
                ).optimise()
            ),
            Self::Seq(_) => {
                RuleCond::Seq(self
                    .resolve_seq(src_dict, trie, resolved_src, resolved_pattern)?
                    .lower()
                )
            }
            Self::Ident(ident) => {
                let seq = resolved_seq.get(ident);
                let pattern = resolved_pattern.get(ident);
                match (seq, pattern) {
                    (Some(seq), None)     => RuleCond::Seq(seq.clone().lower()),
                    (None, Some(pattern)) => RuleCond::Pattern(Predicate::Unit, pattern.clone()),
                    (Some(_), Some(_))    => return Err(Error::Unspanned(format!("Identifier in rule condition cannot refer to both a sequence and a pattern: {}", ident))),
                    (None, None)          => return Err(Error::Unspanned(format!("Unknown identifier in rule condition: {}", ident))),
                }
            }
            Self::Not(boxed) => {
                let (pattern, span) = boxed.as_ref();
                let pred_pattern_match = pattern
                    .resolve_rule_cond(src_dict, trie, resolved_src, resolved_pattern, resolved_seq)
                    .map_err(|err| err.with_span(*span))?;
                if let RuleCond::Pattern(Predicate::Unit, pattern_match) = pred_pattern_match {
                    RuleCond::Pattern(Predicate::Not, pattern_match.optimise())
                } else {
                    return Err(Error::Unspanned(format!(
                        "Expected a pattern in negation, found: {:?}",
                        pattern
                    ))
                    .with_span(*span));
                }
            }
            _ => {
                return Err(Error::Unspanned(format!(
                    "Unexpected expression type for resolving rule: {:?}",
                    self
                )));
            }
        })
    }

    pub fn resolve_rule(
        &self,
        src_dict: &HashMap<SrcName, DomainSrcId>,
        trie: &Trie,
        resolved_src: &HashMap<&'src str, (et_encoding::SrcId, Match)>,
        resolved_pattern: &HashMap<&'src str, Match>,
        resolved_seq: &HashMap<&'src str, SeqTree>,
    ) -> Result<Rule, Error> {
        Ok(match self {
            Self::Rule(conditions) => {
                if conditions.is_empty() {
                    return Err(Error::Unspanned("Rule must have at least one condition".to_string()));
                }
                let resolved_conditions = conditions
                    .iter()
                    .map(|(condition, span)|
                        condition
                            .resolve_rule_cond(src_dict, trie, resolved_src, resolved_pattern, resolved_seq)
                            .map_err(|err| err.with_span(*span))
                    )
                    .collect::<Result<_, _>>()?;
                Rule(resolved_conditions)
            }
            _ => {
                return Err(Error::Unspanned(format!(
                    "Unexpected expression type for resolving rule: {:?}",
                    self
                )));
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use et_core::bits::BitsMatch;

    // Tests for the Match enum and its check method
    #[test]
    fn test_match_x_always_passes() {
        let m = Match::X;
        assert!(m.check(0));
        assert!(m.check(u32::MAX));
        assert!(m.check(0xDEADBEEF));
    }

    #[test]
    fn test_match_bits_exact_value() {
        let bm = BitsMatch {
            mask: 0xFFFF,
            value: 0x1234,
        };
        let m = Match::Bits(bm);

        assert!(m.check(0x00001234));
        assert!(m.check(0xFFFF1234));
        assert!(!m.check(0x00001235));
        assert!(!m.check(0x00001230));
    }

    #[test]
    fn test_match_not() {
        let bm = BitsMatch {
            mask: 0xFF,
            value: 0x42,
        };
        let m = Match::Not(Box::new(Match::Bits(bm)));

        assert!(!m.check(0x00000042));
        assert!(!m.check(0xFFFFFF42));
        assert!(m.check(0x00000000));
        assert!(m.check(0x00000001));
    }

    #[test]
    fn test_match_and_both_required() {
        let bm1 = BitsMatch {
            mask: 0xFF,
            value: 0x42,
        };
        let bm2 = BitsMatch {
            mask: 0xFF00,
            value: 0xAA00,
        };
        let m = Match::And(Box::new(Match::Bits(bm1)), Box::new(Match::Bits(bm2)));

        assert!(m.check(0x0000AA42));
        assert!(!m.check(0x00004200));
        assert!(!m.check(0x0000AA00));
        assert!(!m.check(0x00000042));
    }

    #[test]
    fn test_match_any_passes() {
        let bm1 = BitsMatch {
            mask: 0xFF,
            value: 0x42,
        };
        let bm2 = BitsMatch {
            mask: 0xFF,
            value: 0xAA,
        };
        let m = Match::Any(vec![Match::Bits(bm1), Match::Bits(bm2)]);

        assert!(m.check(0x00000042));
        assert!(m.check(0x000000AA));
        assert!(!m.check(0x00000000));
    }

    #[test]
    fn test_match_optimise_to_x() {
        let bm = BitsMatch {
            mask: 0xFF,
            value: 0x42,
        };
        let m = Match::Any(vec![Match::X, Match::Bits(bm)]);
        let optimised = m.optimise();

        assert!(matches!(optimised, Match::X));
    }

    #[test]
    fn test_match_optimise_removes_redundant_x() {
        let bm = BitsMatch {
            mask: 0xFF,
            value: 0x42,
        };
        let m = Match::And(Box::new(Match::X), Box::new(Match::Bits(bm.clone())));
        let optimised = m.optimise();

        assert_eq!(optimised, Match::Bits(bm));
    }

    #[test]
    fn test_match_optimise_flattens_nested_any() {
        let inner = Match::Any(vec![
            BitsMatch {
                mask: 0xFF,
                value: 0x42,
            }
            .into(),
            BitsMatch {
                mask: 0xFF,
                value: 0xAA,
            }
            .into(),
        ]);
        let m = Match::Any(vec![
            inner,
            BitsMatch {
                mask: 0xFF00,
                value: 0xBB00,
            }
            .into(),
        ]);
        let optimised = m.optimise();

        assert!(matches!(optimised, Match::Any(_)));
        if let Match::Any(inner_vec) = optimised {
            assert_eq!(inner_vec.len(), 3);
        } else {
            panic!("Expected Any after optimisation");
        }
    }

    // Test SeqTree optimisations
    #[test]
    fn test_seqtree_optimise_flatten_nested_seq() {
        let bm1 = BitsMatch {
            mask: 0xFF,
            value: 0x42,
        };
        let bm2 = BitsMatch {
            mask: 0xFF00,
            value: 0xAA00,
        };
        let tree = SeqTree::Seq(vec![
            SeqTree::Pattern(Predicate::Unit, Match::Bits(bm1)),
            SeqTree::Seq(vec![SeqTree::Pattern(
                Predicate::Unit,
                Match::Bits(bm2.clone()),
            )]),
        ]);

        let optimised = tree.optimise();

        if let SeqTree::Seq(items) = optimised {
            assert_eq!(items.len(), 2);
            assert!(matches!(
                items[1],
                SeqTree::Pattern(Predicate::Unit, Match::Bits(_))
            ));
        } else {
            panic!("Expected Seq after optimisation");
        }
    }

    // -------------------------------------------------------
    // Test resolve methods
    // -------------------------------------------------------
    #[test]
    fn test_src_id_resolve() {
        let mut src_dict = HashMap::new();
        src_dict.insert(SrcName::Mat("seawater".to_string()), DomainSrcId::Mat(5));

        assert_eq!(
            SrcId::Mat(5).resolve(&src_dict).unwrap(),
            DomainSrcId::Mat(5)
        );
        assert_eq!(
            SrcId::MatName("seawater").resolve(&src_dict).unwrap(),
            DomainSrcId::Mat(5)
        );
        assert!(SrcId::MatName("unknown").resolve(&src_dict).is_err());
    }

    #[test]
    fn test_resolve_mat_id() {
        let src_id = SrcId::Mat(5);
        let dict = make_test_src_dict();

        let resolved = src_id.resolve(&dict).unwrap();
        assert_eq!(resolved, DomainSrcId::Mat(5));
    }

    #[test]
    fn test_resolve_mat_name() {
        let src_id = SrcId::MatName("seawater");
        let dict = make_test_src_dict();

        let resolved = src_id.resolve(&dict).unwrap();
        assert_eq!(resolved, DomainSrcId::Mat(5));
    }

    #[test]
    fn test_seqtree_optimise_single_element_becomes_pattern() {
        let bm = BitsMatch {
            mask: 0xFF,
            value: 0x42,
        };
        let tree = SeqTree::Seq(vec![SeqTree::Pattern(Predicate::Unit, Match::Bits(bm))]);

        let optimised = tree.optimise();

        assert!(matches!(optimised, SeqTree::Pattern(_, _)));
    }

    fn make_test_src_dict() -> HashMap<SrcName, DomainSrcId> {
        let mut dict = HashMap::new();
        dict.insert(SrcName::Mat("seawater".to_string()), DomainSrcId::Mat(5));
        dict.insert(SrcName::Mat("glass".to_string()), DomainSrcId::Mat(3));
        dict.insert(SrcName::Mat("air".to_string()), DomainSrcId::Mat(0));
        dict.insert(
            SrcName::Surf("TargetToy".to_string()),
            DomainSrcId::Surf(12),
        );
        dict.insert(
            SrcName::Surf("TargetTube".to_string()),
            DomainSrcId::Surf(15),
        );
        dict.insert(
            SrcName::MatSurf("Water:Water_material".to_string()),
            DomainSrcId::MatSurf(7),
        );
        dict.insert(SrcName::Light("laser".to_string()), DomainSrcId::Light(0));
        dict
    }
}
