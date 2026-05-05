//! Parse - Parser for the filter DSL
//!
//! This module provides the parser that converts tokens
//! into the abstract syntax tree (AST).

use chumsky::{input::ValueInput, prelude::*};

use crate::{
    ast::{DeclType, Declaration, Expr, SrcId},
    lexer::Token,
};
use et_core::Repetition;

type Span = SimpleSpan;

/// Creates an expression parser for the filter DSL.
///
/// This is the main parser that converts tokenized input
/// into AST declarations.
///
/// # Type Parameters
///
/// * `'tokens` - Lifetime of tokens
/// * `'src` - Lifetime of source code
/// * `I` - Input type (must implement ValueInput)
///
/// # Returns
///
/// A chumsky parser that produces [`Vec<Declaration>`]
pub fn expr_parser<'tokens, 'src: 'tokens, I>()
-> impl Parser<'tokens, I, Vec<Declaration<'src>>, extra::Err<Rich<'tokens, Token<'src>, Span>>> + Clone
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = Span>,
{
    let src_id_name = select! {Token::SrcId(ty) => ty}
        .then(
            select! {Token::Str(ident) => ident}
                .delimited_by(just(Token::Ctrl('(')), just(Token::Ctrl(')')))
        )
        .map_with(|(ty, ident), e| match SrcId::parse_name(ty, ident) {
            Ok(src_id) => (Expr::SrcId(src_id), e.span()),
            Err(err) => {
                let span = e.span();
                e.emit(chumsky::prelude::Rich::custom(span, err.to_string()));
                (Expr::X, span)
            }
        });
    let src_id_val = select! {Token::SrcId(ty) => ty}
        .then(
            select! {Token::Num(val) => val}
                .delimited_by(just(Token::Ctrl('(')), just(Token::Ctrl(')')))
        )
        .map_with(|(ty, id), e| match SrcId::parse_id(ty, id) {
            Ok(src_id) => (Expr::SrcId(src_id), e.span()),
            Err(err) => {
                let span = e.span();
                e.emit(chumsky::prelude::Rich::custom(span, err.to_string()));
                (Expr::X, span)
            }
        });
    let src_id = src_id_name.or(src_id_val).labelled("source identifier");

    let src_id_any = recursive(|src_id_any| {
        let src_id_items = src_id
            .clone()
            .or(src_id_any.clone())
            .separated_by(just(Token::Ctrl(',')))
            .collect::<Vec<_>>();

        just(Token::Any)
            .ignore_then(src_id_items.delimited_by(just(Token::Ctrl('[')), just(Token::Ctrl(']'))))
            .map_with(|src_id_items, e| (Expr::Any(src_id_items), e.span()))
            .boxed()
    });

    let x = just(Token::X).map_with(|_, e| (Expr::X, e.span()));

    let src_id_value =
        src_id
        .or(src_id_any)
        .or( // Recovery: suggest correct SrcIdType if mistyped as Ident(Number)
            select! { Token::Ident(ident) => ident }
                .then_ignore(just(Token::Ctrl('(')))
                .then(
                    select! {
                        t @ Token::Num(_) => t,
                        t @ Token::Str(_) => t,
                    }
                )
                .then_ignore(just(Token::Ctrl(')')))
                .map_with(|(ident, arg), e| {
                    let span = e.span();
                    if let Some(stripped) = ident.strip_suffix("Id") {
                        // Suggest the correct SrcIdType
                        e.emit(Rich::custom(
                            span,
                            format!(
                                "Unknown source id type `{}`. Did you mean `{}`?\nTry `{}({})` instead of `{}({})`.",
                                ident,
                                stripped,
                                stripped, arg,
                                ident, arg
                            ),
                        ));
                    } else {
                        e.emit(Rich::custom(
                            span,
                            format!(
                                "`{}` is not a valid source id type. Valid types: Mat, Surf, MatSurf, Light, Detector.",
                                ident
                            ),
                        ));
                    }
                    // Still return an error node for the AST
                    (Expr::X, span)
                })
        )
        .or(select!{Token::Ident(ident) => Expr::Ident(ident)}.map_with(|val, e| (val, e.span())))
        .or(x.clone())
        .labelled("SrcId set matching in UID encoding")
        .boxed();

    //let field = select!{
    //        Token::FieldId(f) => Expr::Field(f),
    //        Token::X          => Expr::X,
    //    }
    //    .map_with(|field_expr, e| (field_expr, e.span()))
    //    .labelled("field")
    //    .recover_with(skip_then_retry_until(
    //        any().ignored(),
    //        choice((
    //            just(Token::Concat).ignored(),
    //            just(Token::Ctrl(')')).ignored(),
    //            just(Token::Ctrl(',')).ignored(),
    //        )),
    //    ));

    let field = select! {
            Token::FieldId(f) => Expr::Field(f),
            Token::X          => Expr::X,
        }
        .map_with(|field_expr, e| (field_expr, e.span()))
        .labelled("field");

    let fields = field
        .separated_by(just(Token::Concat))
        .at_least(1)
        .collect::<Vec<_>>();

    let pattern = fields
        .then_ignore(just(Token::Concat))
        .then(src_id_value.clone())
        .map_with(|(fields, src_id), e| {
            let mut all_fields = fields;
            all_fields.push(src_id);
            (Expr::Pattern(all_fields), e.span())
        })
        .boxed()
        .labelled("pattern construction to match event encoding in UID");

    let inline_pattern =
        pattern.clone().or(select! {Token::Ident(ident) => ident}
            .map_with(|ident, e| (Expr::Ident(ident), e.span())));

    let inline_pattern_items = inline_pattern
        .clone()
        .separated_by(just(Token::Ctrl(',')))
        .collect::<Vec<_>>();

    let pattern_any = just(Token::Any)
        .ignore_then(
            inline_pattern_items.delimited_by(just(Token::Ctrl('[')), just(Token::Ctrl(']'))),
        )
        .map_with(|patterns, e| (Expr::Any(patterns), e.span()))
        .labelled("pattern any set");

    let pattern_value = inline_pattern
        .clone()
        .or(pattern_any)
        .or(x)
        .labelled("pattern value")
        .boxed();

    let neg_pattern = just(Token::Predicates('!'))
        .ignore_then(pattern_value.clone())
        .map_with(|p, e| (Expr::Not(Box::new(p)), e.span()))
        .labelled("negated pattern");

    let repetition = select! {
            Token::Predicates('*') => Repetition::ZeroOrMore,
            Token::Predicates('+') => Repetition::OneOrMore,
            Token::Predicates('?') => Repetition::Optional,
        }
        .or(
            just(Token::Ctrl('{'))
                .ignore_then(select! { Token::Num(n) => n as usize }.or_not())
                .then(
                    just(Token::Ctrl(','))
                        .ignore_then(select! { Token::Num(m) => m as usize }.or_not())
                        .or_not()
                )
                .then_ignore(just(Token::Ctrl('}')))
                .map(|(n_opt, m_opt)| match (n_opt, m_opt) {
                    (Some(n), None)          => Repetition::NTimes(n),
                    (Some(n), Some(None))    => Repetition::AtLeast(n),
                    (None, Some(Some(m)))    => Repetition::AtMost(m),
                    (Some(n), Some(Some(m))) => Repetition::Interval(n, m),
                    _                        => Repetition::Unit, // fallback for invalid syntax
                })
        )
        .labelled("repetition operator");

    let repetition_pattern = repetition
        .then(pattern_value.clone())
        .map_with(|(r, p), e| (Expr::Repeat(r, Box::new(p)), e.span()))
        .labelled("repetition pattern");

    let predicated_pattern = repetition_pattern
        .or(neg_pattern.clone())
        .labelled("predicated pattern")
        .or(pattern_value.clone())
        .boxed();

    let seq = recursive(|seq| {
        let seq_items = predicated_pattern
            .clone()
            .or(seq.clone())
            .separated_by(just(Token::Ctrl(',')))
            .allow_trailing()
            .collect::<Vec<_>>();

        just(Token::Seq)
            .ignore_then(just(Token::Ctrl('[')))
            .ignore_then(seq_items)
            .then_ignore(just(Token::Ctrl(']')))
            .map_with(|items, e| (Expr::Seq(items), e.span()))
    })
    .labelled("sequence construction");

    let decl_ledger = just(Token::Ledger)
        .ignore_then(just(Token::Ctrl('=')))
        .ignore_then(
            select! {Token::Str(path) => path}
                .map_with(|path, e| (Expr::LedgerPath(path), e.span())),
        )
        .map_with(|expr, e| Declaration {
            name: "ledger",
            decl_type: DeclType::LedgerPath,
            span: e.span(),
            body: expr,
        })
        .labelled("ledger path declaration")
        .boxed();

    let decl_signals = just(Token::Signals)
        .ignore_then(just(Token::Ctrl('=')))
        .ignore_then(
            select! {Token::Str(path) => path}
                .map_with(|path, e| (Expr::SignalsPath(path), e.span())),
        )
        .map_with(|expr, e| Declaration {
            name: "signals",
            decl_type: DeclType::SignalsPath,
            span: e.span(),
            body: expr,
        })
        .labelled("signals path declaration")
        .boxed();

    let decl_src = just(Token::SrcDecl)
        .ignore_then(select! {Token::Ident(ident) => ident})
        .then_ignore(just(Token::Ctrl('=')))
        .then(src_id_value)
        .map_with(|(name, src_id_val), e| Declaration {
            name,
            decl_type: DeclType::SrcId,
            span: e.span(),
            body: src_id_val,
        })
        .labelled("SrcId declaration")
        .boxed();

    let decl_pattern = just(Token::PatternDecl)
        .ignore_then(select! {Token::Ident(ident) => ident})
        .then_ignore(just(Token::Ctrl('=')))
        .then(pattern)
        .map_with(|(name, pattern), e| Declaration {
            name,
            decl_type: DeclType::Pattern,
            span: e.span(),
            body: pattern,
        })
        .labelled("pattern declaration")
        .boxed();

    let decl_seq = just(Token::SeqDecl)
        .ignore_then(select! {Token::Ident(ident) => ident})
        .then_ignore(just(Token::Ctrl('=')))
        .then(seq.clone())
        .map_with(|(name, seq), e| Declaration {
            name,
            decl_type: DeclType::Sequence,
            span: e.span(),
            body: seq,
        })
        .labelled("sequence declaration")
        .boxed();

    let condition = predicated_pattern.clone().or(seq);

    let condition_items = condition
        .separated_by(just(Token::Ctrl(',')))
        .allow_trailing()
        .collect::<Vec<_>>()
        .map_with(|items, e| (items, e.span()));

    let decl_rule = just(Token::RuleDecl)
        .ignore_then(select! {Token::Ident(ident) => ident})
        .then_ignore(just(Token::Ctrl('=')))
        .then(condition_items.delimited_by(just(Token::Ctrl('{')), just(Token::Ctrl('}'))))
        .map_with(|(name, (items, items_span)), e| Declaration {
            name,
            decl_type: DeclType::Rule,
            span: e.span(),
            body: (Expr::Rule(items), items_span),
        })
        .labelled("rule declaration")
        .boxed();

    let decl = decl_src
        .or(decl_seq)
        .or(decl_pattern)
        .or(decl_rule)
        .or(decl_ledger)
        .or(decl_signals);

    decl.repeated().at_least(1).collect::<Vec<_>>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer;
    use std::collections::HashSet;

    fn default_dict() -> HashSet<String> {
        let mut dict = HashSet::new();
        dict.insert("MCRT".to_string());
        dict.insert("Material".to_string());
        dict.insert("Interface".to_string());
        dict.insert("Elastic".to_string());
        dict.insert("Inelastic".to_string());
        dict.insert("Reflector".to_string());
        dict.insert("Emission".to_string());
        dict.insert("Detection".to_string());
        dict.insert("Backward".to_string());
        dict.insert("Forward".to_string());
        dict
    }

    fn parse_script(src: &str) -> Vec<Declaration<'_>> {
        let dict = default_dict();
        let tokens = lexer(&dict).parse(src).unwrap();
        expr_parser()
            .parse(
                tokens
                    .as_slice()
                    .map((src.len()..src.len()).into(), |(t, s)| (t, s)),
            )
            .into_result()
            .unwrap()
    }

    #[test]
    fn test_parse_ledger_path() {
        let src = r#"ledger = "path/to/ledger.json""#;
        let decls = parse_script(src);
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].name, "ledger");
        assert!(matches!(decls[0].body.0, Expr::LedgerPath(_)));
        assert!(matches!(decls[0].decl_type, DeclType::LedgerPath));
    }

    #[test]
    fn test_parse_signals_path() {
        let src = r#"signals = "path/to/signals.csv""#;
        let decls = parse_script(src);
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].name, "signals");
        assert!(matches!(decls[0].body.0, Expr::SignalsPath(_)));
        assert!(matches!(decls[0].decl_type, DeclType::SignalsPath));
    }

    #[test]
    fn test_parse_src_id_named() {
        let src = r#"src water_id = Mat("seawater")"#;
        let decls = parse_script(src);
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].name, "water_id");
        assert_eq!(decls[0].decl_type, DeclType::SrcId);
    }

    #[test]
    fn test_parse_src_id_numeric() {
        let src = r#"src air_id = Mat(0)"#;
        let decls = parse_script(src);
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].name, "air_id");
        assert_eq!(decls[0].decl_type, DeclType::SrcId);
    }

    #[test]
    fn test_parse_multiple_src_declarations() {
        let src = r#"
            src water_id = Mat("seawater")
            src glass_id = Mat("glass")
            src air_id = Mat(0)
        "#;
        let decls = parse_script(src);
        assert_eq!(decls.len(), 3);
    }
}
