//! Lexer tokens for the filter DSL
//!
//! This module defines the tokens produced by the lexer
//! before parsing into the AST and the parser of the source string
//! into the tokens list.

use std::{collections::HashSet, fmt};

use chumsky::prelude::*;

type Span = SimpleSpan;
type Spanned<T> = (T, Span);

/// Tokens produced by the lexer.
///
/// These tokens represent the atomic units of the filter DSL
/// before parsing into the AST.
#[derive(Debug, Clone, PartialEq)]
pub enum Token<'src> {
    // - "[...]" used for "any[...]", "seq[...]" and "perm[...]"
    // - "{n,m}" used for repetition counts in interval [n,m]
    // - "{ conditions* }" lists conditions to be all met
    // - "," Separator for list, patterns and inside repetition
    // - "=" Assignment operator for field, pattern and seq declarations
    /// Control characters: '=', '[', ']', ',', '{', '}'
    Ctrl(char), // '=', '[', ']', ',' , '{', '}'
    /// Repetition operators: '*', '+', '?', '!'
    Predicates(char), // '*', '+', '?', ! = {0,}, {1,}, {0,1}, 0
    /// '|' Concatenation operator
    Concat,
    /// 'X'=DontCare - Matches any event field or pattern
    X,
    /// "any[" ... "]" matches any field/pattern with the specified values
    Any,
    /// "perm[" ... "]" matches patterns in any permutation/order
    Perm,
    /// - "seq[ ... ]" inline sequence
    Seq,
    /// "src <src_ident> = <expr>"
    SrcDecl,
    /// "pattern `pattern_ident` = `expr`"
    PatternDecl,
    /// - "sequence `seq_ident` = seq?[]" enumerates patterns in sequence order
    SeqDecl,
    /// "rule `rule_ident` = { <expr> }"
    RuleDecl,
    /// Identifiers for src ids, patterns and sequences
    Ident(&'src str),
    /// Field Identifier defined by the encoding specification
    // MCRT, Emission, Detection,
    // Material, Interface, Elastic, etc. => Must define a dictionary of Field names that are
    // reserved
    FieldId(&'src str),
    /// <SrcIdName>(<value>) where value can be hex/dec
    // "match for "MatId", "MatSurfId", "SurfId", "LightId", "DetectorId": <SrcIdName>("<name>") or
    SrcId(&'src str),
    /// String literals
    Str(&'src str),
    /// Numeric literals (e.g., repetition counts, SrcId values)
    Num(u16),
    /// `ledger` keyword
    Ledger,
    /// `signals` keyword
    Signals,
}

impl fmt::Display for Token<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Token::Ctrl(c)       => write!(f, "{c}"     ),
            Token::Predicates(c) => write!(f, "{c}"     ),
            Token::Concat        => write!(f, "|"       ),
            Token::X             => write!(f, "X"       ),
            Token::Any           => write!(f, "any"     ),
            Token::Perm          => write!(f, "perm"    ),
            Token::Seq           => write!(f, "seq"     ),
            Token::SrcDecl       => write!(f, "src"     ),
            Token::PatternDecl   => write!(f, "pattern" ),
            Token::SeqDecl       => write!(f, "sequence"),
            Token::RuleDecl      => write!(f, "rule"    ),
            Token::Ident(s)      => write!(f, "{s}"     ),
            Token::FieldId(s)    => write!(f, "{s}"     ),
            Token::SrcId(s)      => write!(f, "{s}"     ),
            Token::Num(n)        => write!(f, "{n}"     ),
            Token::Str(s)        => write!(f, "\"{s}\"" ),
            Token::Ledger        => write!(f, "ledger"  ),
            Token::Signals       => write!(f, "signals" ),
        }
    }
}

/// The lexer for the Eldritch-Trace filter DSL.
pub fn lexer<'src>(
    dict: &HashSet<String>,
) -> impl Parser<'src, &'src str, Vec<Spanned<Token<'src>>>, extra::Err<Rich<'src, char, Span>>> {
    // A parser for numbers
    // FIXME: Hex parses not working properly
    let hex_num = just("0x")
        .ignore_then(text::int(16))
        .map(|s: &str| u16::from_str_radix(s, 16).unwrap())
        .map(Token::Num);

    let dec_num = text::int(10)
        .to_slice()
        .from_str()
        .unwrapped()
        .map(Token::Num);

    let num = hex_num.or(dec_num);

    // A parser for strings
    let r#str = just('"')
        .ignore_then(none_of('"').repeated().to_slice())
        .then_ignore(just('"'))
        .map(Token::Str);

    // A parser for operators and  control characters (delimiters, semicolons, etc.)
    let ctrl = one_of("=[]{}(),").map(Token::Ctrl);
    let predicate = one_of("*+?!").map(Token::Predicates);
    let concat = just('|').to(Token::Concat);

    // WARN: 'X' is reserved for "don't care", however the character 'X' should be allowed as part
    // of another string
    let dont_care = just('X').to(Token::X);

    // A parser for identifiers and keywords
    let keyword =
               text::ascii::keyword("src"     ).to(Token::SrcDecl    )
           .or(text::ascii::keyword("pattern" ).to(Token::PatternDecl))
           .or(text::ascii::keyword("sequence").to(Token::SeqDecl    ))
           .or(text::ascii::keyword("rule"    ).to(Token::RuleDecl   ))
           .or(text::ascii::keyword("any"     ).to(Token::Any        ))
           .or(text::ascii::keyword("perm"    ).to(Token::Perm       ))
           .or(text::ascii::keyword("seq"     ).to(Token::Seq        ))
           .or(text::ascii::keyword("ledger"  ).to(Token::Ledger     ))
           .or(text::ascii::keyword("signals" ).to(Token::Signals    ));

    let src_id = text::ascii::keyword("Mat")
        .or(text::ascii::keyword("MatSurf"))
        .or(text::ascii::keyword("Surf"))
        .or(text::ascii::keyword("Light"))
        .or(text::ascii::keyword("Detector"))
        .map(Token::SrcId);

    let field_id = text::ascii::ident()
        .filter(move |&s| dict.contains(s))
        .map(Token::FieldId);

    let ident = text::ascii::ident().map(Token::Ident);

    // A single token can be one of the above
    // WARN: Identifiers may not contain 'X' character
    // as that will be parsed as don't care
    let token = num
        .or(r#str)
        .or(ctrl)
        .or(concat)
        .or(predicate)
        .or(keyword)
        .or(src_id)
        .or(field_id)
        .or(dont_care)
        .or(ident);

    let comment = just("#")
        .then(any().and_is(just('\n').not()).repeated())
        .padded();

    token
        .map_with(|tok, e| (tok, e.span()))
        .padded_by(comment.repeated())
        .padded()
        // If we encounter an error, skip and attempt to lex the next character as a token instead
        .recover_with(skip_then_retry_until(any().ignored(), end()))
        .repeated()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[test]
    fn test_decimal_numbers() {
        let dict = default_dict();
        let tokens = lexer(&dict).parse("0 42 123 65535").unwrap();
        assert_eq!(tokens.len(), 4);
        assert!(matches!(tokens[0].0, Token::Num(0)));
        assert!(matches!(tokens[1].0, Token::Num(42)));
        assert!(matches!(tokens[2].0, Token::Num(123)));
        assert!(matches!(tokens[3].0, Token::Num(65535)));
    }

    #[test]
    #[ignore]
    fn test_hex_numbers() {
        let dict = default_dict();
        let tokens = lexer(&dict).parse("0x00 0xFF 0x1A 0xABCD").unwrap();
        assert_eq!(tokens.len(), 4);
        assert!(matches!(tokens[0].0, Token::Num(0)));
        assert!(matches!(tokens[1].0, Token::Num(255)));
        assert!(matches!(tokens[2].0, Token::Num(26)));
        assert!(matches!(tokens[3].0, Token::Num(43981)));
    }

    #[test]
    fn test_simple_strings() {
        let dict = default_dict();
        let tokens = lexer(&dict).parse("\"hello\" \"world\"").unwrap();
        assert_eq!(tokens.len(), 2);
        assert!(matches!(tokens[0].0, Token::Str("hello")));
        assert!(matches!(tokens[1].0, Token::Str("world")));
    }

    #[test]
    fn test_strings_with_spaces() {
        let dict = default_dict();
        let tokens = lexer(&dict)
            .parse("\"seawater\" \"glass material\"")
            .unwrap();
        assert_eq!(tokens.len(), 2);
        assert!(matches!(tokens[0].0, Token::Str("seawater")));
        assert!(matches!(tokens[1].0, Token::Str("glass material")));
    }

    #[test]
    fn test_all_control_chars() {
        let dict = default_dict();
        let tokens = lexer(&dict).parse("=[]{},()").unwrap();
        assert_eq!(tokens.len(), 8);
        assert!(matches!(tokens[0].0, Token::Ctrl('=')));
        assert!(matches!(tokens[1].0, Token::Ctrl('[')));
        assert!(matches!(tokens[2].0, Token::Ctrl(']')));
        assert!(matches!(tokens[3].0, Token::Ctrl('{')));
        assert!(matches!(tokens[4].0, Token::Ctrl('}')));
        assert!(matches!(tokens[5].0, Token::Ctrl(',')));
        assert!(matches!(tokens[6].0, Token::Ctrl('(')));
        assert!(matches!(tokens[7].0, Token::Ctrl(')')));
    }

    #[test]
    fn test_concat_operator() {
        let dict = default_dict();
        let tokens = lexer(&dict).parse("|").unwrap();
        assert_eq!(tokens.len(), 1);
        assert!(matches!(tokens[0].0, Token::Concat));
    }

    #[test]
    fn test_all_predicates() {
        let dict = default_dict();
        let tokens = lexer(&dict).parse("*+?!").unwrap();
        assert_eq!(tokens.len(), 4);
        assert!(matches!(tokens[0].0, Token::Predicates('*')));
        assert!(matches!(tokens[1].0, Token::Predicates('+')));
        assert!(matches!(tokens[2].0, Token::Predicates('?')));
        assert!(matches!(tokens[3].0, Token::Predicates('!')));
    }

    #[test]
    fn test_all_keywords() {
        let dict = default_dict();
        let tokens = lexer(&dict)
            .parse("src pattern sequence rule any perm seq ledger signals")
            .unwrap();
        assert_eq!(tokens.len(), 9);
        assert!(matches!(tokens[0].0, Token::SrcDecl));
        assert!(matches!(tokens[1].0, Token::PatternDecl));
        assert!(matches!(tokens[2].0, Token::SeqDecl));
        assert!(matches!(tokens[3].0, Token::RuleDecl));
        assert!(matches!(tokens[4].0, Token::Any));
        assert!(matches!(tokens[5].0, Token::Perm));
        assert!(matches!(tokens[6].0, Token::Seq));
        assert!(matches!(tokens[7].0, Token::Ledger));
        assert!(matches!(tokens[8].0, Token::Signals));
    }

    #[test]
    fn test_all_src_id_types() {
        let dict = default_dict();
        let tokens = lexer(&dict)
            .parse("Mat Surf MatSurf Light Detector")
            .unwrap();
        assert_eq!(tokens.len(), 5);
        assert!(matches!(tokens[0].0, Token::SrcId("Mat")));
        assert!(matches!(tokens[1].0, Token::SrcId("Surf")));
        assert!(matches!(tokens[2].0, Token::SrcId("MatSurf")));
        assert!(matches!(tokens[3].0, Token::SrcId("Light")));
        assert!(matches!(tokens[4].0, Token::SrcId("Detector")));
    }

    #[test]
    fn test_field_ids_from_dict() {
        let mut dict = default_dict();
        dict.insert("EventType".to_string());

        let tokens = lexer(&dict)
            .parse("MCRT Material Elastic EventType")
            .unwrap();
        assert_eq!(tokens.len(), 4);
        assert!(matches!(tokens[0].0, Token::FieldId("MCRT")));
        assert!(matches!(tokens[1].0, Token::FieldId("Material")));
        assert!(matches!(tokens[2].0, Token::FieldId("Elastic")));
        assert!(matches!(tokens[3].0, Token::FieldId("EventType")));
    }

    #[test]
    fn test_x_dont_care() {
        let dict = default_dict();
        let tokens = lexer(&dict).parse("X").unwrap();
        assert_eq!(tokens.len(), 1);
        assert!(matches!(tokens[0].0, Token::X));
    }

    #[test]
    fn test_user_identifiers() {
        let dict = default_dict();
        let tokens = lexer(&dict)
            .parse("water_id glass_interf my_pattern rule_name")
            .unwrap();
        assert_eq!(tokens.len(), 4);
        assert!(matches!(tokens[0].0, Token::Ident("water_id")));
        assert!(matches!(tokens[1].0, Token::Ident("glass_interf")));
        assert!(matches!(tokens[2].0, Token::Ident("my_pattern")));
        assert!(matches!(tokens[3].0, Token::Ident("rule_name")));
    }

    #[test]
    fn test_single_line_comment() {
        let dict = default_dict();
        let tokens = lexer(&dict).parse("# This is a comment\nX").unwrap();
        assert_eq!(tokens.len(), 1);
        assert!(matches!(tokens[0].0, Token::X));
    }

    #[test]
    fn test_comment_at_end_of_line() {
        let dict = default_dict();
        let tokens = lexer(&dict).parse("X # comment").unwrap();
        assert_eq!(tokens.len(), 1);
        assert!(matches!(tokens[0].0, Token::X));
    }

    #[test]
    fn test_multiple_comments() {
        let dict = default_dict();
        let tokens = lexer(&dict)
            .parse("# comment 1\nX # comment 2\n# comment 3")
            .unwrap();
        assert_eq!(tokens.len(), 1);
        assert!(matches!(tokens[0].0, Token::X));
    }

    #[test]
    fn test_spaces() {
        let dict = default_dict();
        let tokens = lexer(&dict).parse("  X   Z  ").unwrap();
        // Just verify tokens exist and have some content
        assert!(!tokens.is_empty());
    }

    #[test]
    fn test_tabs() {
        let dict = default_dict();
        let tokens = lexer(&dict).parse("\tX\tZ\t").unwrap();
        assert_eq!(tokens.len(), 2);
    }

    #[test]
    fn test_newlines() {
        let dict = default_dict();
        let tokens = lexer(&dict).parse("X\nY\nZ").unwrap();
        assert_eq!(tokens.len(), 3);
    }

    #[test]
    fn test_pattern_with_concat() {
        let mut dict = default_dict();
        dict.insert("MCRT".to_string());
        dict.insert("Material".to_string());
        dict.insert("Elastic".to_string());

        let src = "MCRT | Material | Elastic | X | Mat(5)";
        let tokens = lexer(&dict).parse(src).unwrap();

        assert_eq!(tokens.len(), 12);
        assert!(matches!(tokens[0].0, Token::FieldId("MCRT")));
        assert!(matches!(tokens[1].0, Token::Concat));
        assert!(matches!(tokens[2].0, Token::FieldId("Material")));
        assert!(matches!(tokens[3].0, Token::Concat));
        assert!(matches!(tokens[4].0, Token::FieldId("Elastic")));
        assert!(matches!(tokens[5].0, Token::Concat));
        assert!(matches!(tokens[6].0, Token::X));
        assert!(matches!(tokens[7].0, Token::Concat));
        assert!(matches!(tokens[8].0, Token::SrcId("Mat")));
        assert!(matches!(tokens[9].0, Token::Ctrl('(')));
        assert!(matches!(tokens[10].0, Token::Num(5)));
    }

    #[test]
    fn test_any_with_multiple_items() {
        let mut dict = default_dict();
        dict.insert("Mat".to_string());
        dict.insert("MatSurf".to_string());

        let src = "any[Mat(\"seawater\"), MatSurf(\"Water:Water_material\")]";
        let tokens = lexer(&dict).parse(src).unwrap();

        assert!(tokens.len() > 5);
        assert!(matches!(tokens[0].0, Token::Any));
        assert!(matches!(tokens[1].0, Token::Ctrl('[')));
    }

    #[test]
    fn test_ctrl_display() {
        assert_eq!(Token::Ctrl('=').to_string(), "=");
        assert_eq!(Token::Ctrl('[').to_string(), "[");
        assert_eq!(Token::Ctrl(']').to_string(), "]");
        assert_eq!(Token::Concat.to_string(), "|");
    }

    #[test]
    fn test_predicates_display() {
        assert_eq!(Token::Predicates('*').to_string(), "*");
        assert_eq!(Token::Predicates('+').to_string(), "+");
        assert_eq!(Token::Predicates('?').to_string(), "?");
        assert_eq!(Token::Predicates('!').to_string(), "!");
    }

    #[test]
    fn test_keywords_display() {
        assert_eq!(Token::Any.to_string(), "any");
        assert_eq!(Token::Perm.to_string(), "perm");
        assert_eq!(Token::Seq.to_string(), "seq");
        assert_eq!(Token::SeqDecl.to_string(), "sequence");
        assert_eq!(Token::SrcDecl.to_string(), "src");
        assert_eq!(Token::PatternDecl.to_string(), "pattern");
        assert_eq!(Token::RuleDecl.to_string(), "rule");
        assert_eq!(Token::Ledger.to_string(), "ledger");
        assert_eq!(Token::Signals.to_string(), "signals");
    }
}
