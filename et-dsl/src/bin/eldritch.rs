//! Eldritch-Trace DSL command-line tool

use aetherus_events::{
    Uid,
    read::{CsvRecord, read_csv, read_ledger},
};
use anyhow::{Context, Result};
use clap::Parser;
use env_logger::Env;
use et_dsl::{extract_ledger_path, extract_signals_path, model::resolve_ast, parse_script};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use log::info;

/// Eldritch-Trace DSL command-line tool
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Path to the encoding scheme file
    #[arg(short, long)]
    encoding: String,
    /// Write `.dot` graphviz to visualise encoding Trie
    #[arg(long, default_value_t = false)]
    emit_trie: bool,
    /// Optional path to the ledger file (overrides script declaration)
    #[arg(short, long)]
    ledger: Option<String>,
    /// Optional path to the signals file (overrides script declaration)
    #[arg(short, long)]
    signals: Option<String>,
    /// Optional output directory for generated DOT files (defaults to ledger directory)
    #[arg(short, long)]
    output_dir: Option<String>,
    /// Top N most common UIDs to visualize in the DOT graph (default: 0)
    #[arg(short, long, default_value_t = 0)]
    top: usize,
    /// Verbosity level for logging (e.g., "info", "debug", "error")
    #[arg(short, long, default_value = "info")]
    verbose: String,
    /// Dry run without evaluating rules (for testing parsing and resolution)
    #[arg(long, default_value_t = false)]
    dry: bool,
    /// Path to the DSL script file
    script: String,
}

fn main() -> Result<()> {
    let args = Args::parse();

    env_logger::Builder::from_env(Env::default().default_filter_or(args.verbose))
        .format_timestamp(None)
        .init();

    if let Some(output_dir) = &args.output_dir {
        std::fs::create_dir_all(Path::new(output_dir))
            .context("Failed to create output directory")?;
    }

    let encoding_filename = args.encoding;
    let encoding_filepath = Path::new(&encoding_filename);
    let encoding_src =
        &fs::read_to_string(encoding_filepath).context("Failed to read encoding scheme file")?;

    let script_arg = args.script;
    let script_filepath = Path::new(&script_arg);
    let script_src = fs::read_to_string(script_filepath).context("Failed to read script file")?;

    // 1. Build the decoder Trie from the encoding scheme
    let trie = et_encoding::build_decoder(encoding_src)
        .context("Failed to build decoder from encoding scheme")?;

    if args.emit_trie {
        let dot_file = if let Some(output_dir) = &args.output_dir {
            Path::new(output_dir).join(format!(
                "{}_trie.dot",
                encoding_filepath.file_stem().unwrap().to_str().unwrap()
            ))
        } else {
            encoding_filepath
                .parent()
                .unwrap_or_else(|| {
                    panic!(
                        "Failed to get parent directory of encoding path: {:?}",
                        encoding_filepath
                    )
                })
                .join(format!(
                    "{}_trie.dot",
                    encoding_filepath.file_stem().unwrap().to_str().unwrap()
                ))
        };
        let graphviz_dot = trie.emit_dot();
        std::fs::write(&dot_file, graphviz_dot).context("Failed to write DOT file")?;
        info!("Emitted Trie DOT file: {:?}", dot_file);
    };

    // 2. Extract the field dictionary from the Trie for use in parsing the script
    let dict = trie.get_fields();
    info!("FieldId dictionary: {:?}", dict);

    // 3. Parse the script into declarations: src, pattern, sequence, rule
    let declarations = parse_script(&script_src, &dict);

    // 4. Extract ledger path and signals path from declarations (or use command-line overrides)
    // FIXME: Combine with the arguments parsed with clap
    let ledger_path = if let Some(filepath) = args.ledger {
        PathBuf::from(filepath)
    } else {
        extract_ledger_path(&declarations, &script_src, script_filepath).unwrap_or_else(|| panic!("Failed to extract ledger from: {}", script_filepath.display()))
    };
    let signals_path = if let Some(filepath) = args.signals {
        Some(PathBuf::from(filepath))
    } else {
        extract_signals_path(&declarations, &script_src, script_filepath)
    };

    info!(
        "Start resolving with ledger={:?}, signals={:?}",
        ledger_path, signals_path
    );

    // 5. Read the ledger and resolve the declarations from source values allocated in the ledger
    // and pattern encoding specified in the Trie
    let ledger = read_ledger(&ledger_path).expect("Failed to read ledger file");
    let src_dict = ledger.get_src_dict();

    info!("SrcId dictionary from ledger: {:?}", src_dict);

    let rules = resolve_ast(&script_src, &declarations, &src_dict, &trie);

    info!(
        "Finished resolving with Rules: {:#?}",
        rules
            .keys()
            .map(|key| key.to_string())
            .collect::<Vec<_>>()
    );

    let dot_dirname = if let Some(ref dirname) = args.output_dir {
        PathBuf::from(dirname)
    } else {
        ledger_path
            .parent()
            .unwrap_or_else(|| {
                panic!(
                    "Failed to get parent directory of ledger path: {:?}",
                    ledger_path
                )
            })
            .to_path_buf()
    };

    // 6. Rank top N most common UIDs in the signals file and emit a DOT graph visualizing their relationships in the ledger
    if let Some(ref signals_path) = signals_path
        && (args.top > 0)
    {
        info!(
            "Ranking top {} most common UIDs in signals file: {:?}",
            args.top, signals_path
        );
        let mut uid_hist = HashMap::new();
        let signals = read_csv(signals_path).context("Failed to read signals file")?;
        for signal in signals {
            *uid_hist.entry(signal.uid).or_insert(0_usize) += 1;
        }
        // Get top most common UIDs
        let mut uid_hist_vec: Vec<_> = uid_hist.into_iter().collect();
        uid_hist_vec.sort_by_key(|(_uid, count)| *count);

        let uids_top = uid_hist_vec
            .into_iter()
            .rev()
            .take(args.top)
            .map(|(uid, count)| (Uid::decode(uid), count))
            .collect::<HashMap<_, _>>();
        if args.top < 50 {
            info!(
                "Top {} most common UIDs in signals: {:#?}",
                args.top, uids_top
            );
        }
        let dot_file = dot_dirname.join(format!(
            "{}_top.dot",
            script_filepath.file_stem().unwrap().to_str().unwrap()
        ));
        let graphviz_dot = ledger.emit_dot_with_freq(uids_top.keys(), &uids_top);
        std::fs::write(&dot_file, graphviz_dot).context("Failed to write DOT file")?;
    }

    if args.dry {
        info!("Dry run enabled, skipping rule evaluation");
        return Ok(());
    }

    // 7. Evaluate each rule on the ledger and emit a DOT graph visualizing the UIDs that match the rule
    for (rule_name, rule) in rules.iter() {
        print!("{:<40}", format!("Rule: \x1b[32m{}\x1b[0m", rule_name));
        let uids = rule.evaluate(&ledger)?;
        println!("Found {} UIDs", uids.len());

        if let Some(ref signals_path) = signals_path {
            let signals = read_csv(signals_path).context("Failed to read signals file")?;
            let hex_uids = uids.iter().map(|uid| uid.encode()).collect::<Vec<u64>>();

            let signals_filtered = signals
                .iter()
                .filter(|record| hex_uids.contains(&record.uid))
                .collect::<Vec<&CsvRecord>>();

            println!(
                "Matching signals for rule \x1b[32m{}\x1b[0m: len={} from {}",
                rule_name,
                signals_filtered.len(),
                signals.len()
            );

            let dirpath = if let Some(ref output_dir) = args.output_dir {
                PathBuf::from(output_dir)
            } else {
                signals_path.parent().unwrap().to_path_buf()
            };
            let csv_outpath = dirpath.join(format!(
                "{}_{}.csv",
                signals_path.file_stem().unwrap().to_str().unwrap(),
                rule_name
            ));

            let mut csv_writer =
                csv::Writer::from_path(csv_outpath).expect("Unable to create output CSV file");
            for filtered_record in signals_filtered {
                csv_writer
                    .serialize(filtered_record)
                    .expect("Unable to write filtered CSV file");
            }
        }

        //println!("Found UIDS: {:#?}", uids);

        let dot_file = dot_dirname.join(format!(
            "{}_{}.dot",
            script_filepath.file_stem().unwrap().to_str().unwrap(),
            rule_name
        ));
        let graphviz_dot = ledger.emit_dot(&uids);
        std::fs::write(&dot_file, graphviz_dot).context("Failed to write DOT file")?;
    }

    Ok(())
}
