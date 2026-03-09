use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;
use std::process;

use clap::Parser;
use dp_library::weight::{
    WeightConfig, WeightScheme, classify, compute_weights_multi_pass, parse_e_file,
    rake_classified_full,
};
use dp_library::RflFile;
use ipf_survey::RakingConfig;

#[derive(Parser, Debug)]
#[command(
    name = "weight",
    about = "Compute raking weights from UNCLE weight tables"
)]
struct Args {
    /// Control table ID (e.g., 600)
    table_id: u16,

    #[arg(short = 'e', long = "e-file", help = "Path to .E file")]
    exec_file: Option<String>,

    #[arg(short = 'd', long = "data-file", help = "Path to data file (.fin, .rft, .c)")]
    data_file: Option<String>,

    #[arg(short = 'l', long = "layout-file", help = "Path to .rfl file")]
    layout_file: Option<String>,

    #[arg(short = 'o', long = "output", help = "Output file path (default: <stem>.WT)")]
    output: Option<String>,
}

fn find_file_with_extension(dir: &str, extensions: &[&str]) -> Option<String> {
    let path = Path::new(dir);
    if let Ok(entries) = std::fs::read_dir(path) {
        let mut sorted_entries: Vec<_> = entries.flatten().collect();
        sorted_entries.sort_unstable_by(|a, b| {
            b.metadata()
                .unwrap()
                .len()
                .cmp(&a.metadata().unwrap().len())
        });
        for entry in sorted_entries {
            if let Some(file_name) = entry.file_name().to_str() {
                for ext in extensions {
                    if file_name.to_lowercase().ends_with(ext) {
                        return Some(file_name.to_string());
                    }
                }
            }
        }
    }
    None
}

fn resolve_file(arg: Option<&String>, extensions: &[&str], label: &str) -> String {
    if let Some(path) = arg {
        if !Path::new(path).exists() {
            eprintln!("{label} not found: {path}");
            process::exit(1);
        }
        return path.clone();
    }
    find_file_with_extension(".", extensions).unwrap_or_else(|| {
        eprintln!("Could not auto-discover {label} in current directory");
        process::exit(1);
    })
}

fn read_data_lines(path: &str) -> Vec<String> {
    let file = File::open(path).unwrap_or_else(|e| {
        eprintln!("Could not open data file {path}: {e}");
        process::exit(1);
    });
    let reader = BufReader::new(file);
    reader
        .lines()
        .map(|r| {
            r.unwrap_or_else(|e| {
                eprintln!("Error reading data file: {e}");
                process::exit(1);
            })
        })
        .collect()
}

fn format_weight(value: f64, width: usize, decimal: usize) -> String {
    format!("{value:0>width$.decimal$}")
}

fn punch_weight(line: &str, weight_str: &str, col_start: usize, col_end: usize) -> String {
    let start_idx = col_start - 1; // 1-based → 0-based
    let end_idx = col_end; // col_end is 1-based inclusive, so byte index = col_end

    // Ensure the line is long enough.
    let mut buf = if line.len() < end_idx {
        let mut s = line.to_string();
        s.extend(std::iter::repeat_n(' ', end_idx - s.len()));
        s
    } else {
        line.to_string()
    };

    buf.replace_range(start_idx..end_idx, weight_str);
    buf
}

fn default_output_name(data_file: &str) -> String {
    let stem = Path::new(data_file)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output");
    format!("{stem}.WT")
}

fn main() {
    let args = Args::parse();

    // Resolve files.
    let exec_file = resolve_file(args.exec_file.as_ref(), &[".e"], ".E file");
    let layout_file = args.layout_file.clone()
        .or_else(|| find_file_with_extension(".", &[".rfl"]));
    let data_file = args.data_file.clone().unwrap_or_else(|| {
        find_file_with_extension(".", &[".c"])
            .or_else(|| find_file_with_extension(".", &[".fin"]))
            .unwrap_or_else(|| {
                eprintln!("Could not auto-discover data file in current directory");
                process::exit(1);
            })
    });
    let output_file = args
        .output
        .clone()
        .unwrap_or_else(|| default_output_name(&data_file));

    eprintln!("Exec file:   {exec_file}");
    if let Some(ref lf) = layout_file {
        eprintln!("Layout file: {lf}");
    }
    eprintln!("Data file:   {data_file}");
    eprintln!("Output file: {output_file}");

    // Parse .E file.
    let spec = parse_e_file(Path::new(&exec_file), args.table_id).unwrap_or_else(|e| {
        eprintln!("Error parsing .E file: {e}");
        process::exit(1);
    });

    let has_qualifiers = spec.passes.iter().any(|p| p.qualifier.is_some());

    // Print pass/directive summary.
    for (pi, pass) in spec.passes.iter().enumerate() {
        let d = &pass.directive;
        let table_ids_str: Vec<String> = d.table_ids.iter().map(std::string::ToString::to_string).collect();
        if has_qualifiers {
            if let Some(ref qual) = pass.qualifier {
                eprintln!("\nPass {}: qualifier = {qual:?}", pi + 1);
            } else {
                eprintln!("\nPass {} (unqualified):", pi + 1);
            }
        }
        eprintln!("\nWeight tables: {}", table_ids_str.join(", "));
        eprintln!(
            "Output location: cols {}-{} ({} decimal places, {} chars wide)",
            d.col_start, d.col_end, d.decimal_width, d.field_width()
        );
        if let Some(total) = d.total {
            eprintln!("TOTAL scaling: {total}");
        }
    }

    // Print table summary.
    for table in &spec.tables {
        let target_sum: f64 = table.categories.iter().map(|c| c.target).sum();
        eprintln!(
            "  Table {:>3}: {:>2} categories, targets sum = {:.2}",
            table.id,
            table.categories.len(),
            target_sum
        );
    }

    // Load RFL (optional — only needed for CFMC conditions or base_weight_field).
    let rfl = layout_file.map(|lf| {
        RflFile::from_file(&lf).unwrap_or_else(|e| {
            eprintln!("Error reading layout file: {e}");
            process::exit(1);
        })
    });
    let data = read_data_lines(&data_file);
    eprintln!("Records: {}", data.len());

    // Capture output location from the first pass (all passes should target the
    // same output columns in split-sample designs).
    let col_start = spec.passes[0].directive.col_start;
    let col_end = spec.passes[0].directive.col_end;
    let decimal_width = spec.passes[0].directive.decimal_width;
    let field_width = spec.passes[0].directive.field_width();

    if !spec.assignments.is_empty() {
        eprintln!("\nColumn assignments:");
        for a in &spec.assignments {
            eprintln!("  X IF({:?}) col {}={}", a.condition, a.col, a.value);
        }
    }

    // Move assignments out so spec.tables can be moved into scheme later.
    let assignments = spec.assignments;

    let base_config = WeightConfig {
        raking: RakingConfig {
            convergence: ipf_survey::ConvergenceConfig {
                max_iterations: 40,
                tolerance: 1e-6,
                ..Default::default()
            },
            ..Default::default()
        },
        base_weight_field: None,
        target_tolerance: Some(0.01),
    };

    // weights: Option<f64> per record. None = not qualified (don't punch weight).
    let weights: Vec<Option<f64>> = if has_qualifiers {
        // Qualified weighting: each pass weights its qualifying subset independently.
        eprintln!("\nRunning {} qualified pass(es)...", spec.passes.len());
        compute_weights_multi_pass(
            &spec.passes,
            &spec.tables,
            &base_config,
            rfl.as_ref(),
            &data,
        )
        .unwrap_or_else(|e| {
            eprintln!("\nWeighting error: {e}");
            process::exit(1);
        })
    } else {
        // Single-pass: use the detailed classify + rake_full path for diagnostics.
        let normalization = spec.passes[0].directive.total.map_or(
            ipf_survey::Normalization::SumToN,
            ipf_survey::Normalization::SumTo,
        );
        let scheme = WeightScheme {
            tables: spec.tables,
            config: WeightConfig {
                raking: RakingConfig {
                    normalization,
                    ..base_config.raking
                },
                ..base_config
            },
        };

        let (survey, targets) = classify(&scheme, rfl.as_ref(), &data).unwrap_or_else(|e| {
            eprintln!("\nClassification error: {e}");
            process::exit(1);
        });

        let result =
            rake_classified_full(&survey, &targets, &scheme.config.raking).unwrap_or_else(|e| {
                eprintln!("\nRaking error: {e}");
                process::exit(1);
            });

        if result.convergence.converged {
            eprintln!(
                "Raking converged in {} iterations (residual: {:.2e})",
                result.convergence.iterations, result.convergence.final_residual
            );
        } else {
            eprintln!(
                "Warning: raking did not converge after {} iterations (residual: {:.2e})",
                result.convergence.iterations, result.convergence.final_residual
            );
        }

        result.weights.into_iter().map(Some).collect()
    };

    // Weight statistics (only for raked records).
    let raked: Vec<f64> = weights.iter().filter_map(|w| *w).collect();
    let sum: f64 = raked.iter().sum();
    let min = raked.iter().copied().fold(f64::INFINITY, f64::min);
    let max = raked.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    #[allow(clippy::cast_precision_loss)]
    let mean = sum / raked.len() as f64;
    eprintln!("\nWeighted records: {} / {}", raked.len(), data.len());
    eprintln!("Weight range: {min:.4} - {max:.4}");
    eprintln!("Weight mean:  {mean:.4}");
    eprintln!("Weight sum:   {sum:.2}");

    // Write output.
    let out = File::create(&output_file).unwrap_or_else(|e| {
        eprintln!("Could not create output file {output_file}: {e}");
        process::exit(1);
    });
    let mut writer = BufWriter::new(out);

    for (line, weight) in data.iter().zip(weights.iter()) {
        // Only punch the weight field for records that were raked.
        let mut output_line = weight.map_or_else(
            || line.clone(),
            |w| {
                let weight_str = format_weight(w, field_width, decimal_width);
                if weight_str.len() > field_width {
                    eprintln!(
                        "Warning: formatted weight '{weight_str}' exceeds field width {field_width}"
                    );
                }
                punch_weight(line, &weight_str, col_start, col_end)
            },
        );

        // Apply X IF column assignments (runs after raking, so assignments
        // are not overwritten by the weight punch).
        for a in &assignments {
            if a.condition.evaluate(&output_line).unwrap_or(false) {
                let idx = a.col - 1; // 1-based → 0-based
                let end = idx + a.value.len();
                if output_line.len() < end {
                    output_line.extend(std::iter::repeat_n(' ', end - output_line.len()));
                }
                output_line.replace_range(idx..end, &a.value);
            }
        }

        writeln!(writer, "{output_line}").unwrap_or_else(|e| {
            eprintln!("Error writing output: {e}");
            process::exit(1);
        });
    }

    eprintln!("\nDone. Wrote {} records to {output_file}", data.len());
}
