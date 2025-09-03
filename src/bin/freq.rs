use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Result as IoResult};
use std::path::Path;

use clap::Parser;
use dp_library::{RflFile, RflQuestion};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Question labels to generate frequencies for
    questions: Vec<String>,

    #[arg(
        short = 'l',
        long = "layoutfile",
        help = "The layout file to use, ie p0001.rfl"
    )]
    layout_file: Option<String>,

    #[arg(
        short = 'd',
        long = "datafile",
        help = "The data file to use, ie p0001.fin"
    )]
    data_file: Option<String>,

    #[arg(short = 'v', long = "verbose", help = "Verbose output")]
    _verbose: bool,

    #[arg(
        short = 'w',
        long = "weight",
        help = "Weight specification: start_col.width (e.g., 1000.7 for column 1000, width 7)"
    )]
    weight: Option<String>,
}

fn find_file_with_extension(dir: &str, extensions: &[&str]) -> Option<String> {
    let path = Path::new(dir);
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
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

#[derive(Debug)]
struct WeightSpec {
    start_col: usize,
    width: usize,
}

impl WeightSpec {
    fn parse(weight_spec: &str) -> Result<Self, String> {
        let parts: Vec<&str> = weight_spec.split('.').collect();
        if parts.len() != 2 {
            return Err(
                "Weight specification must be in format start_col.width (e.g., 1000.7)".to_string(),
            );
        }

        let start_col = parts[0]
            .parse::<usize>()
            .map_err(|_| "Invalid start column number".to_string())?;
        let width = parts[1]
            .parse::<usize>()
            .map_err(|_| "Invalid width number".to_string())?;

        if start_col == 0 {
            return Err("Column numbers are 1-indexed, start_col must be >= 1".to_string());
        }
        if width == 0 {
            return Err("Width must be >= 1".to_string());
        }

        Ok(Self { start_col, width })
    }

    fn extract_weight(&self, line: &str) -> f64 {
        let start_idx = self.start_col - 1; // Convert to 0-indexed
        let end_idx = start_idx + self.width;

        if start_idx >= line.len() {
            return 1.0; // Default weight if line is too short
        }

        let end_idx = end_idx.min(line.len());
        let weight_str = &line[start_idx..end_idx];

        weight_str.trim().parse::<f64>().unwrap_or(1.0)
    }
}

struct FrequencyStats {
    punch_counts: HashMap<String, f64>,
    valid_cases: f64,
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
fn calculate_percentage(count: f64, total: f64) -> u32 {
    if total == 0.0 {
        0
    } else {
        (count / total).mul_add(100.0, 0.5).floor() as u32
    }
}

fn print_question_frequency(
    question: &RflQuestion,
    stats: &FrequencyStats,
    _total_lines: usize,
    total_weight: f64,
) {
    println!("{}:", question.label.to_uppercase());

    let main_text = question.main_text();
    if !main_text.is_empty() {
        println!("{}\n", main_text.join("\n\n"));
    }

    let mut total_responses = 0.0;

    // Sort response codes for consistent output
    let mut sorted_punches: Vec<_> = question.response_codes.iter().collect();
    sorted_punches.sort_by_key(|(code, _)| code.parse::<i32>().unwrap_or(999));

    for (punch_code, response_text) in sorted_punches {
        let count = stats.punch_counts.get(punch_code).unwrap_or(&0.0);
        let percentage = calculate_percentage(*count, total_weight);

        println!("{punch_code:>4} {response_text:<64} {count:>5.0} {percentage:>3}%");

        total_responses += count;
    }

    println!();

    // Print summary statistics
    let total_pct = calculate_percentage(total_responses, total_weight);
    println!(
        "     {:<64} {:>5.0} {:>3}%",
        "TOTAL RESPONSES", total_responses, total_pct
    );

    let valid_pct = calculate_percentage(stats.valid_cases, total_weight);
    println!(
        "     {:<64} {:>5.0} {:>3}%",
        "VALID CASES", stats.valid_cases, valid_pct
    );

    let missing_weight = total_weight - stats.valid_cases;
    let missing_pct = calculate_percentage(missing_weight, total_weight);
    println!(
        "     {:<64} {:>5.0} {:>3}%",
        "MISSING CASES", missing_weight, missing_pct
    );

    println!(
        "     {:<64} {:>5.0} {:>3}%",
        "TOTAL CASES", total_weight, 100
    );
    println!();
}

fn main() -> IoResult<()> {
    let args = Args::parse();

    // Determine layout file
    let layout_filename = args.layout_file.map_or_else(
        || {
            find_file_with_extension(".", &[".rfl"]).unwrap_or_else(|| {
                eprintln!("Could not find rfl file in the current directory");
                std::process::exit(1);
            })
        },
        |file| file,
    );

    // Determine data file
    let data_filename = args.data_file.map_or_else(
        || {
            find_file_with_extension(".", &[".fin"])
                .or_else(|| find_file_with_extension(".", &[".rft"]))
                .unwrap_or_else(|| {
                    eprintln!("Could not find fin or rft file in the current directory");
                    std::process::exit(1);
                })
        },
        |file| file,
    );

    // Verify files exist
    if !Path::new(&layout_filename).exists() {
        eprintln!("{layout_filename} not found in the current directory");
        std::process::exit(1);
    }
    if !Path::new(&data_filename).exists() {
        eprintln!("{data_filename} not found in the current directory");
        std::process::exit(1);
    }

    println!("Using data file     {data_filename}");
    println!("Using layout file   {layout_filename}");

    // Parse weight specification if provided
    let weight_spec = args
        .weight
        .as_ref()
        .map(|weight_str| match WeightSpec::parse(weight_str) {
            Ok(spec) => {
                println!(
                    "Using weights from column {} width {}",
                    spec.start_col, spec.width
                );
                spec
            }
            Err(e) => {
                eprintln!("Error parsing weight specification: {e}");
                std::process::exit(1);
            }
        });

    // Load RFL file
    let rfl_file = RflFile::from_file(&layout_filename)?;
    let questions_map = rfl_file.questions();

    // Determine which questions to process
    let mut questions_to_process = Vec::new();

    if args.questions.is_empty() {
        // No questions specified - process all questions from RFL
        for question in rfl_file.questions_array() {
            questions_to_process.push(question.label.clone());
        }
    } else {
        // Validate specified questions exist in RFL
        for question_label in &args.questions {
            let question_label = question_label.to_uppercase();
            if !questions_map.contains_key(&question_label) {
                eprintln!("Could not find question label {question_label} in RFL.");
                std::process::exit(1);
            }
            questions_to_process.push(question_label);
        }
    }

    // Initialize frequency statistics for all questions
    let mut all_stats: HashMap<String, FrequencyStats> = HashMap::new();
    for question_label in &questions_to_process {
        if let Some(question) = questions_map.get(question_label) {
            let mut punch_counts = HashMap::new();
            // Initialize punch counts from RFL definition
            for punch_code in question.response_codes.keys() {
                punch_counts.insert(punch_code.clone(), 0.0);
            }
            all_stats.insert(
                question_label.clone(),
                FrequencyStats {
                    punch_counts,
                    valid_cases: 0.0,
                },
            );
        }
    }

    // Process data file once
    let data_file = File::open(&data_filename)?;
    let reader = BufReader::new(data_file);
    let mut line_count = 0;
    let mut total_weight = 0.0;
    for line in reader.lines() {
        let line = line?;

        // Extract weight for this line
        let weight = weight_spec
            .as_ref()
            .map_or(1.0, |spec| spec.extract_weight(&line));
        total_weight += weight;

        // Process each question for this line
        for question_label in &questions_to_process {
            if let Some(question) = questions_map.get(question_label) {
                let responses = question.responses(&line);
                let mut has_response = false;

                for response in &responses {
                    let response = response.trim();
                    if !response.is_empty()
                        && let Some(stats) = all_stats.get_mut(question_label)
                    {
                        *stats
                            .punch_counts
                            .entry(response.to_string())
                            .or_insert(0.0) += weight;
                        has_response = true;
                    }
                }
                if has_response && let Some(stats) = all_stats.get_mut(question_label) {
                    stats.valid_cases += weight;
                }
            }
        }

        line_count += 1;
        if line_count % 100 == 0 {
            print!(".");
            std::io::Write::flush(&mut std::io::stdout()).unwrap();
        }
    }
    println!();

    // Output frequency tables for each question
    for question_label in &questions_to_process {
        if let Some(question) = questions_map.get(question_label)
            && let Some(stats) = all_stats.get(question_label)
        {
            print_question_frequency(question, stats, line_count, total_weight);
        }
    }

    Ok(())
}
