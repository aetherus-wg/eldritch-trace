//! This module provides functionality to parse encoding definitions from markdown tables.

use crate::trie::{Encoding, Field};
use anyhow::anyhow;
use anyhow::{Context, Result};
use et_core::bits::{BitsMatch, BitsRange};
use itertools::Itertools;
use markdown_ppp::ast::Block;
use markdown_ppp::ast::Inline;
use markdown_ppp::parser::MarkdownParserState;
use markdown_ppp::parser::parse_markdown;
use std::str::FromStr;

fn unwrap_inline(inline: &Inline) -> Result<String> {
    match inline {
        Inline::Text(s) => Ok(s.clone()),
        _ => Err(anyhow!("Expected text, received {:?}", inline)),
    }
}

/// Parses a markdown string containing encoding definitions in tables and returns a vector of Encoding.
pub fn parse_encodings(src: &str) -> Result<Vec<Encoding>> {
    let state = MarkdownParserState::new();
    let document =
        parse_markdown(state, src).map_err(|e| anyhow!("Failed to parse markdown: {:?}", e))?;

    let mut encodings = Vec::new();

    // Regex parse to extract name and optional attr from
    // "<name> {<attr>}" or "<name>{<attr>}"
    let re = regex::Regex::new(r"(\w*)(?:\s*\{([^}]*)\})?").unwrap();

    for block in document.blocks {
        if let Block::Table(table) = block {
            let bits_ranges: Vec<BitsRange> = table.rows[0]
                .iter()
                .map(|c| match &c[0] {
                    Inline::Text(s) => BitsRange::from_str(s),
                    _ => Err(anyhow!("Expected text in header cell, received: {:?}", c[0])),
                })
                .collect::<Result<Vec<_>, _>>()?; // header row

            for (fields_md, encodings_md) in table.rows.iter().skip(1).tuples() {
                let mut specified = true;
                let mut fields_encoding = Vec::new();
                for (bit_range, (field_md, encoding_md)) in bits_ranges
                    .iter()
                    .zip(fields_md.iter().zip(encodings_md.iter()))
                {
                    let field_str = unwrap_inline(&field_md[0])?;
                    let encoding = BitsMatch::parse(&unwrap_inline(&encoding_md[0])?, bit_range)
                        .context("Failed to parse bits match")?;
                    if field_str.starts_with('_') && field_str.len() > 1 {
                        specified = false;
                    }

                    let field = if let Some(caps) = re.captures(&field_str) {
                        let name = caps.get(1).unwrap().as_str().to_string();
                        let attr = caps.get(2).map(|m| m.as_str().to_string());
                        if attr == Some("SrcId".to_string()) {
                            Field::from_str(name.as_str())?
                        } else if name == "_" || name == "X" || name.is_empty() {
                            Field::X {
                                size: bit_range.size(),
                                attr,
                            }
                        } else {
                            Field::Named {
                                name,
                                attr,
                                bits: encoding,
                                size: bit_range.size(),
                            }
                        }
                    } else {
                        return Err(anyhow!("Invalid field name format: {}", field_str));
                    };
                    fields_encoding.push(field);
                }
                if specified {
                    encodings.push(Encoding(fields_encoding));
                }
            }
        }
    }
    Ok(encodings)
}

/// Find Encoding with attribute "Direction" which is specified
/// and replace all instances for the X (don't care) with the same attribute specified
pub fn resolved_dir_encodings(encodings: &[Encoding]) -> Vec<Encoding> {
    let dir_fields: Vec<Field> = encodings
        .iter()
        .map(|Encoding(fields_encoding)| fields_encoding)
        .filter_map(|fields_encoding|
            fields_encoding.iter().find(|&field|
                matches!(field, Field::Named{name, attr, bits: _, size: _} if attr.as_deref() == Some("Direction"))).cloned()
        )
        .collect();
    encodings
        .iter()
        .flat_map(|Encoding(fields_encoding)| {
            fields_encoding
                .iter()
                .enumerate()
                .filter_map(|(pos, field)|
                    matches!(
                        field,
                        Field::X {attr, size: field_size} if attr.as_deref() == Some("Direction")
                    )
                    .then_some(pos)
                )
                .flat_map({
                    let dir_fields_captured = dir_fields.clone();
                    move |pos| {
                        dir_fields_captured
                            .clone()
                            .into_iter()
                            .map(move |dir_field| {
                                assert_eq!(dir_field.size(), fields_encoding[pos].size());
                                let mut new_encoding = fields_encoding.clone();
                                new_encoding[pos] = dir_field.clone();
                                Encoding(new_encoding)
                            })
                    }
                })
        })
        .collect()
}

/// Get a list of all field names from the encoding definiton, which can be supplied
/// to the DSL parser as valid FieldId names
pub fn extract_fields(encodings: &[Encoding]) -> Vec<String> {
    encodings
        .iter()
        .flat_map(|Encoding(fields_encoding)| {
            fields_encoding.iter().filter_map(|field| match field {
                    Field::Named{name, attr: _, bits: _, size: _} => Some(name.clone()),
                    _ => None,
                }
            )
        })
        .unique()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_markdown_table() {
        let src = r#"
# Heading

| 31:28 | 27:24    | 15:0            |
|-------|----------|-----------------|
| MCRT  | Emission | LightId {SrcId} |
| 0b0011| 0b0001   | _               |
"#;
        let encodings = parse_encodings(src).unwrap();
        assert_eq!(encodings.len(), 1);
    }

    #[test]
    fn parse_multiple_encodings() {
        let src = r#"
| 31:28    | 27:24 | 15:0              |
|----------|-------|-------------------|
| MCRT     | _     | MatSurfId {SrcId} |
| 0b0011   | 0bxx  | _                 |
| Emission | _     | LightId {SrcId}   |
| 0b0001   | _     | _                 |
"#;
        let encodings = parse_encodings(src).unwrap();
        assert!(encodings.len() >= 2);
    }

    #[test]
    fn parse_underscore_row_ignored() {
        let src = r#"
| 31:28  | 15:0              |
|--------|-------------------|
| MCRT   | MatSurfId {SrcId} |
| 0b0011 | _                 |
"#;
        let encodings = parse_encodings(src).unwrap();
        assert_eq!(encodings.len(), 1);
    }

    #[test]
    fn extract_fields_basic() {
        let src = r#"
| 31:28  | 27:24    | 15:0            |
|--------|----------|-----------------|
| MCRT   | Emission | LightId {SrcId} |
| 0b0011 | 0b0001   | _               |
"#;
        let encodings = parse_encodings(src).unwrap();
        let fields = extract_fields(&encodings);
        assert!(fields.contains(&"MCRT".to_string()));
        assert!(fields.contains(&"Emission".to_string()));
    }

    #[test]
    fn parse_invalid_bits_range() {
        let src = r#"
| invalid | 15:0            |
|---------|-----------------|
| MCRT    | LightId {SrcId} |
"#;
        let result = parse_encodings(src);
        assert!(result.is_err());
    }

    #[test]
    fn parse_complex_encoding() {
        let src = r#"
| 31:24  | 23:22    | 21:20      | 15:0          |
|--------|----------|------------|---------------|
| MCRT   | Material | Absorption | MatId {SrcId} |
| 0b0011 | 0b10     | 0b00       | _             |
"#;
        let encodings = parse_encodings(src).unwrap();
        assert_eq!(encodings.len(), 1);
        let encoding = &encodings[0];
        assert_eq!(encoding.0.len(), 4);
    }

    #[test]
    fn parse_binary_with_x() {
        let src = r#"
| 23:16      | 15:0 |
|------------|------|
| EventType  | _    |
| 0b0001xxxx | _    |
"#;
        let encodings = parse_encodings(src).unwrap();
        assert_eq!(encodings.len(), 1);
    }

    #[test]
    fn parse_hex_value() {
        let src = r#"
| 15:0 | 15:0  |
|------|-------|
| Id   | SrcId |
| 0x2F | _     |
"#;
        let encodings = parse_encodings(src);
        assert!(encodings.is_ok());
    }

    #[test]
    fn resolved_dir_encodings_basic() {
        let src = r#"
| 31:28  | 27:24    | 17:16   | 15:0          |
|--------|----------|---------|---------------|
| MCRT   | Material | Forward | MatId {SrcId} |
| 0b0011 | 0b10     | 0b01    | _             |
"#;
        let encodings = parse_encodings(src).unwrap();
        assert_eq!(encodings.len(), 1);
    }

    #[test]
    fn extract_fields_no_duplicates() {
        let src = r#"
| 31:28  | 27:24    | 15:0            |
|--------|----------|-----------------|
| MCRT   | Emission | LightId {SrcId} |
| 0b0011 | 0b0001   | _               |
| MCRT   | Material | MatId {SrcId}   |
| 0b0011 | 0b10     | _               |
"#;
        let encodings = parse_encodings(src).unwrap();
        let fields = extract_fields(&encodings);
        assert_eq!(fields.iter().filter(|f| *f == "MCRT").count(), 1);
    }
}
