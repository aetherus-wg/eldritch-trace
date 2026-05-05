//! AST - Abstract Syntax Tree types for the filter DSL
//!
//! This module defines the types that represent parsed filter scripts.
//! These types form the intermediate representation before conversion
//! to the semantic model.

use std::collections::HashMap;

use chumsky::prelude::*;

use aetherus_events::{SrcId as DomainSrcId, ledger::SrcName};
use et_core::Repetition;

use crate::error::Error;

pub(crate) type Span = SimpleSpan;
pub(crate) type Spanned<T> = (T, Span);

/// Represents an event source identifier.
///
/// Source IDs identify where events originate from in the simulation.
/// They can be either resolved (numeric encoding) or unresolved (named).
#[derive(Debug, Clone)]
pub(crate) enum SrcId<'src> {
    /// No source specified (matches any source)
    None,
    // Resolved
    /// Material source with numeric ID
    Mat(u16),
    /// Surface source with numeric ID
    Surf(u16),
    /// Material-Surface source with numeric ID
    MatSurf(u16),
    /// Light/Emitter source with numeric ID
    Light(u16),
    /// Detector source with numeric ID
    Detector(u16),
    // To look up
    /// Material source by name (to be resolved)
    MatName(&'src str),
    /// Surface source by name (to be resolved)
    SurfName(&'src str),
    /// Material-Surface source by name (to be resolved)
    MatSurfName(&'src str),
    /// Light/Emitter source by name (to be resolved)
    LightName(&'src str),
    /// Detector source by name (to be resolved)
    DetectorName(&'src str),
}

macro_rules! get_src_id {
    ($subt:ident, $name:expr, $dict:expr) => {
        match $dict.get(&SrcName::$subt($name)) {
            Some(src_id) => Ok(src_id.clone()),
            None => Err(Error::Unspanned(format!(
                "Unknown source name: {}(\"{}\")",
                stringify!($subt),
                $name
            ))),
        }
    };
}

impl<'a> SrcId<'a> {
    pub fn parse_id(src_id_type: &str, id: u16) -> Result<Self, Error> {
        match src_id_type {
            "Mat"              => Ok(Self::Mat(id)),
            "Surf"             => Ok(Self::Surf(id)),
            "MatSurf"          => Ok(Self::MatSurf(id)),
            "Light"            => Ok(Self::Light(id)),
            "Detector" | "Det" => Ok(Self::Detector(id)),
            _ => Err(Error::Unspanned(format!("Unknown source id type: {}", src_id_type))),
        }
    }
    pub fn parse_name(src_id_type: &str, name: &'a str) -> Result<Self, Error> {
        match src_id_type {
            "Mat"              => Ok(Self::MatName(name)),
            "Surf"             => Ok(Self::SurfName(name)),
            "MatSurf"          => Ok(Self::MatSurfName(name)),
            "Light"            => Ok(Self::LightName(name)),
            "Detector" | "Det" => Ok(Self::DetectorName(name)),
            _ => Err(Error::Unspanned(format!("Unknown source id type: {}", src_id_type))),
        }
    }
    pub fn resolve(&self, dict: &HashMap<SrcName, DomainSrcId>) -> Result<DomainSrcId, Error> {
        Ok(match self {
            Self::None            => DomainSrcId::None,
            Self::Mat(n)          => DomainSrcId::Mat(*n),
            Self::Surf(n)         => DomainSrcId::Surf(*n),
            Self::MatSurf(n)      => DomainSrcId::MatSurf(*n),
            Self::Light(n)        => DomainSrcId::Light(*n),
            Self::Detector(n)     => DomainSrcId::Detector(*n),
            Self::MatName(n)      => get_src_id!(Mat, n.to_string(), dict)?,
            Self::SurfName(n)     => get_src_id!(Surf, n.to_string(), dict)?,
            Self::MatSurfName(n)  => get_src_id!(MatSurf, n.to_string(), dict)?,
            Self::LightName(n)    => get_src_id!(Light, n.to_string(), dict)?,
            Self::DetectorName(n) => get_src_id!(Detector, n.to_string(), dict)?,
        })
    }
}

#[derive(Debug, Clone)]
/// Represents an expression in the filter DSL.
///
/// Expressions are the core building blocks of patterns and rules.
/// They define what events to match and how to combine matching criteria.
pub(crate) enum Expr<'src> {
    /// Don't care / wildcard (matches anything)
    X,
    /// User-defined identifier
    Ident(&'src str),
    /// Event field (e.g., Material, Elastic)
    Field(&'src str),
    /// Path to ledger file
    LedgerPath(&'src str),
    /// Path to signals file
    SignalsPath(&'src str),
    /// Match any of several patterns
    Any(Vec<Spanned<Self>>),

    /// Negation
    Not(Box<Spanned<Self>>),
    /// Repetition modifier
    Repeat(Repetition, Box<Spanned<Self>>),
    /// Sequence (ordered)
    Seq(Vec<Spanned<Self>>),
    /// Permutation (any order)
    Perm(Vec<Spanned<Self>>),
    /// Rule condition
    Rule(Vec<Spanned<Self>>), // e.g. (repetition, pattern), seq, pattern, !pattern
    /// Pattern (alternation via `|`)
    Pattern(Vec<Spanned<Self>>), // e.g. MCRT | Material | Elastic | X | water_id
    /// Source ID reference
    SrcId(SrcId<'src>),
}

/// The type of a declaration.
///
/// Indicates what kind of declaration a [`Declaration`] contains.
#[derive(Debug, PartialEq)]
pub enum DeclType {
    /// SrcId declaration to match values described
    SrcId,
    /// Pattern declaration as concatenation of fields
    Pattern,
    /// Sequence declaration as ordered combination of patterns
    Sequence,
    /// Rule declaration as combination of patterns and sequence conditions
    Rule,
    /// Path of the ledger file to load
    LedgerPath,
    /// Path of the signals file to load
    SignalsPath,
}

#[derive(Debug)]
/// A declaration in the filter DSL.
///
/// Declarations are the top-level statements in a filter script,
/// defining sources, patterns, sequences, and rules.
pub struct Declaration<'src> {
    /// The name of the declaration
    pub name: &'src str,
    /// The type of declaration
    pub decl_type: DeclType,
    /// Source span for error reporting
    pub span: Span,
    /// The declaration body (expression)
    pub(crate) body: Spanned<Expr<'src>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_src_id_to_encoding_id() {
        assert!(matches!(
            SrcId::Mat(5).into(),
            et_encoding::SrcId::MatId
        ));
        assert!(matches!(
            SrcId::MatName("test").into(),
            et_encoding::SrcId::MatId
        ));
        assert!(matches!(
            SrcId::Surf(3).into(),
            et_encoding::SrcId::SurfId
        ));
        assert!(matches!(
            SrcId::SurfName("test").into(),
            et_encoding::SrcId::SurfId
        ));
        assert!(matches!(
            SrcId::MatSurf(1).into(),
            et_encoding::SrcId::MatSurfId
        ));
        assert!(matches!(
            SrcId::MatSurfName("test").into(),
            et_encoding::SrcId::MatSurfId
        ));
        assert!(matches!(
            SrcId::Light(0).into(),
            et_encoding::SrcId::LightId
        ));
        assert!(matches!(
            SrcId::LightName("laser").into(),
            et_encoding::SrcId::LightId
        ));
        assert!(matches!(
            SrcId::Detector(0).into(),
            et_encoding::SrcId::DetectorId
        ));
        assert!(matches!(
            SrcId::DetectorName("sensor").into(),
            et_encoding::SrcId::DetectorId
        ));
    }

    #[test]
    fn test_src_id_parse_id() {
        assert!(matches!(SrcId::parse_id("Mat", 5).unwrap(), SrcId::Mat(5)));
        assert!(matches!(
            SrcId::parse_id("Surf", 10).unwrap(),
            SrcId::Surf(10)
        ));
        assert!(matches!(
            SrcId::parse_id("MatSurf", 3).unwrap(),
            SrcId::MatSurf(3)
        ));
        assert!(matches!(
            SrcId::parse_id("Light", 1).unwrap(),
            SrcId::Light(1)
        ));
        assert!(matches!(
            SrcId::parse_id("Detector", 0).unwrap(),
            SrcId::Detector(0)
        ));
        assert!(matches!(
            SrcId::parse_id("Det", 2).unwrap(),
            SrcId::Detector(2)
        ));
        assert!(SrcId::parse_id("Invalid", 5).is_err());
    }

    #[test]
    fn test_src_id_parse_name() {
        assert!(matches!(
            SrcId::parse_name("Mat", "seawater").unwrap(),
            SrcId::MatName("seawater")
        ));
        assert!(matches!(
            SrcId::parse_name("Surf", "TargetToy").unwrap(),
            SrcId::SurfName("TargetToy")
        ));
        assert!(matches!(
            SrcId::parse_name("MatSurf", "Water:Water_material").unwrap(),
            SrcId::MatSurfName("Water:Water_material")
        ));
        assert!(matches!(
            SrcId::parse_name("Light", "laser").unwrap(),
            SrcId::LightName("laser")
        ));
        assert!(matches!(
            SrcId::parse_name("Detector", "sensor").unwrap(),
            SrcId::DetectorName("sensor")
        ));
        assert!(SrcId::parse_name("Invalid", "name").is_err());
    }
}
