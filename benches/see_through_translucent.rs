use anyhow::{Context, Result};
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use flate2::read::GzDecoder;
use std::{
    hint::black_box,
    env, fs,
    path::{Path, PathBuf},
};
use tar::Archive;

use aetherus_events::{prelude::*, read::read_ledger};

fn get_benches_dir() -> PathBuf {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(manifest_dir)
}

fn decompress_ledger(benches_dir: &Path) -> Result<()> {
    let data_dir = benches_dir.join("data");
    let tar_gz_path = data_dir.join("simulation_ledger.tar.gz");
    let output_path = data_dir.join("simulation_ledger.json");

    if output_path.exists() {
        return Ok(());
    }

    let tar_gz = fs::File::open(tar_gz_path).context("Failed to open simulation_ledger.tar.gz")?;
    let tar = GzDecoder::new(tar_gz);
    let mut archive = Archive::new(tar);
    archive
        .unpack(&data_dir)
        .context("Failed to extract simulation_ledger.tar.gz")?;
    Ok(())
}

fn criterion_benchmark(c: &mut Criterion) {
    let benches_dir = get_benches_dir();

    // Decompress ledger once before benchmarking
    decompress_ledger(&benches_dir).expect("Failed to decompress ledger");

    let data_dir = benches_dir.join("data");
    let encoding_path = data_dir.join("encoding_spec.md");
    let script_path = data_dir.join("bench_script.et");
    let ledger_path = data_dir.join("simulation_ledger.json");

    let encoding_src = fs::read_to_string(encoding_path).expect("Failed to read encoding spec");
    let script_src = fs::read_to_string(script_path).expect("Failed to read script file");

    // Benchmark 1: Build Trie from encoding spec
    c.bench_function("build_trie", |b| {
        b.iter(|| {
            let _trie = et_encoding::build_decoder(black_box(&encoding_src))
                .expect("Failed to build decoder");
        })
    });

    // Build trie once for subsequent benchmarks
    let trie = et_encoding::build_decoder(&encoding_src).expect("Failed to build decoder");

    // Get field dictionary
    let field_dict = trie.get_fields();

    // Benchmark 2: Parse DSL script
    c.bench_function("parse_script", |b| {
        b.iter(|| {
            let _declarations =
                et_dsl::parse_script(black_box(&script_src), black_box(&field_dict));
        })
    });

    // Parse script once for subsequent benchmarks
    let declarations = et_dsl::parse_script(&script_src, &field_dict);

    // Read ledger for semantic model resolution
    let ledger =
        read_ledger(&ledger_path).expect("Failed to read ledger file");
    let ledger_tree: LedgerTree = ledger.into();
    let src_dict = ledger_tree.get_src_dict();

    // Benchmark 3: Resolve declarations and build semantic model
    c.bench_function("resolve_ast", |b| {
        b.iter(|| {
            let _rules = et_dsl::model::resolve_ast(
                black_box(&script_src),
                black_box(&declarations),
                black_box(&src_dict),
                black_box(&trie),
            );
        })
    });

    // Resolve rules once for the main benchmark
    let rules = et_dsl::model::resolve_ast(&script_src, &declarations, &src_dict, &trie);

    // Benchmark 4: Main benchmark - scan ledger to find UIDs matching rules
    let mut group = c.benchmark_group("rule_evaluation");
    for (rule_name, rule) in &rules {
        group.bench_with_input(
            BenchmarkId::new("evaluate_rule", rule_name),
            rule,
            |b, rule| {
                b.iter(|| {
                    let _uids = rule
                        .evaluate(black_box(&ledger_tree))
                        .expect("Failed to evaluate rule");
                })
            },
        );
    }
    group.finish();
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
