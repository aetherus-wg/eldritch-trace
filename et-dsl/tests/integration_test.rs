#![cfg_attr(test, allow(missing_docs))]

use aetherus_events::{
    read::read_ledger,
    prelude::*
};
use anyhow::{Context, Result};
use env_logger;
use et_dsl::{extract_ledger_path, model::resolve_ast, parse_script};
use std::{
    collections::HashMap, env, fs, path::Path
};

use log::info;

#[test]
fn test_integration() -> Result<()> {
    env_logger::init();

    let encoding_filepath = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("spec/encoding_spec.md");
    let encoding_src =
        &fs::read_to_string(encoding_filepath).context("Failed to read encoding scheme file")?;

    let script_filepath = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/analyse.et");
    let script_src = fs::read_to_string(&script_filepath).context("Failed to read script file")?;

    // 1. Build the decoder Trie from the encoding scheme
    let trie = et_encoding::build_decoder(encoding_src)
        .context("Failed to build decoder from encoding scheme")?;

    // 2. Extract the field dictionary from the Trie for use in parsing the script
    let dict = trie.get_fields();
    info!("FieldId dictionary: {:?}", dict);

    // 3. Parse the script into declarations: src, pattern, sequence, rule
    let declarations = parse_script(&script_src, &dict);

    // 4. Extract ledger path and signals path from declarations (or use command-line overrides)
    // FIXME: Combine with the arguments parsed with clap
    let ledger_path =
        extract_ledger_path(&declarations, &script_src, &script_filepath).unwrap_or_else(|| panic!("Failed to extract ledger from: {}", script_filepath.display()));

    info!("Start resolving with ledger={:?}", ledger_path);

    // 5. Read the ledger and resolve the declarations from source values allocated in the ledger
    // and pattern encoding specified in the Trie
    let ledger = read_ledger(&ledger_path).expect("Failed to read ledger file");
    let ledger_tree: LedgerTree = ledger.into();
    let src_dict = ledger_tree.get_src_dict();

    info!("SrcId dictionary from ledger: {:?}", src_dict);

    let rules = resolve_ast(&script_src, &declarations, &src_dict, &trie);

    info!(
        "Finished resolving with Rules: {:#?}",
        rules
            .keys()
            .map(|key| key.to_string())
            .collect::<Vec<_>>()
    );

    let expect_hits: HashMap<&str, usize> = vec![
        ("target_reflect", 0),
        ("plate_reflect", 0),
        ("simple_detect", 54),
        ("plate_subsurface_scatter", 0),
    ].iter().cloned().collect();


    // 7. Evaluate each rule on the ledger and emit a DOT graph visualizing the UIDs that match the rule
    for (rule_name, rule) in rules.iter() {
        print!("{:<40}", format!("Rule: \x1b[32m{}\x1b[0m", rule_name));
        let uids = rule.evaluate(&ledger_tree)?;
        let expected_uids_no = *expect_hits.get(rule_name.as_str()).unwrap_or(&0);
        assert_eq!(uids.len(), expected_uids_no , "Unexpected number of hits for rule \"{}\"", rule_name);
    }

    Ok(())
}
