//! `eldritch-dsl` - A domain-specific language for filtering event data
//!
//! This crate provides a DSL for defining filters, patterns, and rules
//! for processing event data, particularly for MCRT and LIDAR simulations.
//!
//! ## Key Concepts
//!
//! - **Sources**: Define event sources with identifiers (e.g. `Mat("water")`,
//!   `Mat(0)`, `MatSurf(0xFFFE)`)
//! - **Patterns**: Define matching criteria for event types formed of field concatenations
//!   (e.g., `MCRT | Material | Elastic | X | etc.`)
//! - **Sequences**: Define ordered lists of events to match
//! - **Rules**: Define conditions that must be satisfied for matching
//!
//! ## Example
//!
//! ```rust
//! use et_dsl::{parse_script, ast::DeclType};
//!
//! let script = r#"
//!     src water = Mat("seawater")
//!     pattern water_interaction = Material | Elastic | X | water
//! "#;
//! let dict = ["MCRT", "Material", "Elastic"].iter().map(|s| s.to_string()).collect();
//! let decls = parse_script(script, &dict);
//!
//! for decl in &decls {
//!     println!("{}: {:?}", decl.name, decl.decl_type);
//! }
//! ```
//!
//! ## Modules
//!
//! - [`ast`] - Abstract syntax tree types
//! - [`error`] - Error types
//! - [`evaluate`] - Rule evaluation
//! - [`model`] - Semantic model
//! - [`parse`] - Parser implementation
//! - [`mod@lexer`] - Lexer/Tokenizer

pub mod ast;
pub mod error;
pub mod evaluate;
pub mod lexer;
pub mod model;
pub mod parse;

use crate::{
    ast::{Declaration, Expr},
    lexer::lexer,
    parse::expr_parser,
};
use ariadne::{Color, Label, Report, ReportKind, sources};
use chumsky::{Parser, error::Rich, input::Input, span::SimpleSpan};
use itertools::Itertools;
use log::debug;
use std::{collections::HashSet, path::Path};

// -----------------------------------------------
// Model traits and helpers
// -----------------------------------------------
/// A trait for checking if a value matches a pattern.
///
/// This trait is implemented by types that can verify whether
/// a given value satisfies matching criteria.
///
/// # Example
///
/// ```ignored
/// FIXME: This example is ignored because BitsMatch provinence is in encodinc_spec,
/// but it should be move in a common core crate, after we restructure in a workspace
/// use et_dsl::model::{Match, BitsMatch};
/// use et_dsl::Check;
///
/// let bm = BitsMatch { mask: 0xFF, value: 0x42 };
/// let m = Match::Bits(bm);
///
/// assert!(m.check(0x42));
/// assert!(!m.check(0x00));
/// ```
pub trait Check<T> {
    /// Check if the given value matches this pattern.
    fn check(&self, value: T) -> bool;
}

/// Extract the ledger path from parsed declarations.
///
/// Searches through declarations for a `ledger = "path"` statement
/// and returns the resolved path relative to the script file location.
///
/// # Arguments
///
/// * `declarations` - The parsed declarations from a filter script
/// * `script_src` - The original source code (for error reporting)
/// * `script_filepath` - Path to the script file (for resolving relative paths)
///
/// # Returns
///
/// The resolved absolute path to the ledger file, or `None` if not found
pub fn extract_ledger_path(
    declarations: &Vec<Declaration>,
    script_src: &str,
    script_filepath: &Path,
) -> Option<std::path::PathBuf> {
    let script_dirname = &script_filepath
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    let mut ledger_path = None;
    for decl in declarations.iter() {
        if let (Expr::LedgerPath(path), _span) = decl.body {
            if let Some((_, first_span)) = ledger_path {
                failure(
                    "Multiple ledger/photons paths specified".to_string(),
                    ("another declaration here".to_string(), decl.span),
                    [("first declaration here".to_string(), first_span)],
                    script_src,
                );
            } else {
                ledger_path = Some((script_dirname.join(path), decl.span));
            }
        }
    }

    ledger_path.map(|(path, _)| path)
}

/// Extract the signals path from parsed declarations.
///
/// Searches through declarations for a `signals = "path"` statement
/// and returns the resolved path relative to the script file location.
///
/// # Arguments
///
/// * `declarations` - The parsed declarations from a filter script
/// * `script_src` - The original source code (for error reporting)
/// * `script_filepath` - Path to the script file (for resolving relative paths)
///
/// # Returns
///
/// The resolved absolute path to the signals file, or `None` if not found
pub fn extract_signals_path(
    declarations: &Vec<Declaration>,
    script_src: &str,
    script_filepath: &Path,
) -> Option<std::path::PathBuf> {
    let script_dirname = &script_filepath
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    let mut signals_path = None;
    for decl in declarations.iter() {
        if let (Expr::SignalsPath(path), _span) = decl.body {
            if let Some((_, first_span)) = signals_path {
                failure(
                    "Multiple signals paths specified".to_string(),
                    ("another declaration here".to_string(), decl.span),
                    [("first declaration here".to_string(), first_span)],
                    script_src,
                );
            } else {
                signals_path = Some((script_dirname.join(path), decl.span));
            }
        }
    }

   signals_path.map(|(path, _)| path)
}

// -----------------------------------------------
// Parsing entry point
// -----------------------------------------------

/// Parse a filter DSL script into declarations.
///
/// This is the main entry point for parsing filter scripts.
/// It tokenizes the source code and parses it into a sequence of declarations.
///
/// # Arguments
///
/// * `script_src` - The filter DSL script source code
/// * `field_dict` - A set of valid field names for the domain
///
/// # Returns
///
/// A vector of declarations parsed from the script
///
/// # Example
///
/// ```rust
/// use et_dsl::parse_script;
///
/// let script = r#"
///     src water = Mat("seawater")
///     pattern interaction = Material | Elastic | water
///     rule forward = {
///       Material | Elastic | water,
///     }
/// "#;
/// let field_dict = ["MCRT", "Material", "Elastic"].iter().map(|s| s.to_string()).collect();
///
/// let decls = parse_script(script, &field_dict);
/// for decl in &decls {
///     println!("{}: {:?}", decl.name, decl.decl_type);
/// }
/// ```
pub fn parse_script<'src>(
    script_src: &'src str,
    field_dict: &HashSet<String>,
) -> Vec<Declaration<'src>> {
    let tokens = lexer(field_dict)
        .parse(script_src)
        .into_result()
        .unwrap_or_else(|errs| {
            // NOTE: We only need to take the first failure,
            // but we might show all failures before exiting if it's necessary
            if let Some(err) = errs.first() {
                parse_failure(err, script_src);
            }
            std::process::exit(1)
        });

    debug!("Tokens: {}", tokens.iter().map(|t| t.0.to_string()).join(" "));

    expr_parser()
        .parse(
            tokens
                .as_slice()
                .map((script_src.len()..script_src.len()).into(), |(t, s)| (t, s)),
        )
        .into_result()
        .unwrap_or_else(|errs| {
            if let Some(err) = errs.first() {
                parse_failure(err, script_src);
            }
            std::process::exit(1)
        })
}

// -----------------------------------------------
// Parsing helpers
// -----------------------------------------------

/// Report a custom failure with optional extra labels.
///
/// This function prints an error message with source location information
/// and exits the program with exit code 1.
///
/// # Arguments
///
/// * `msg` - The main error message
/// * `label` - Primary error location (message, span)
/// * `extra_labels` - Additional context labels
/// * `src` - Source code for display
pub fn failure(
    msg: String,
    label: (String, SimpleSpan),
    extra_labels: impl IntoIterator<Item = (String, SimpleSpan)>,
    src: &str,
) -> ! {
    let fname = "example";
    Report::build(ReportKind::Error, (fname, label.1.into_range()))
        .with_config(ariadne::Config::new().with_index_type(ariadne::IndexType::Byte))
        .with_message(&msg)
        .with_label(
            Label::new((fname, label.1.into_range()))
                .with_message(label.0)
                .with_color(Color::Red),
        )
        .with_labels(extra_labels.into_iter().map(|label2| {
            Label::new((fname, label2.1.into_range()))
                .with_message(label2.0)
                .with_color(Color::Yellow)
        }))
        .finish()
        .print(sources([(fname, src)]))
        .unwrap();
    std::process::exit(1)
}

/// Parse a chumsky parse error and report it.
///
/// Converts a chumsky [`Rich`] error to a user-friendly error message
/// and exits the program.
pub fn parse_failure(err: &Rich<impl std::fmt::Display>, src: &str) -> ! {
    failure(
        err.reason().to_string(),
        (
            err.found()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "end of input".to_string()),
            *err.span(),
        ),
        err.contexts()
            .map(|(l, s)| (format!("while parsing this {l}"), *s)),
        src,
    )
}
