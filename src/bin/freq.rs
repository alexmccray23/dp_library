use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Result as IoResult};
use std::path::Path;

use clap::Parser;
use library::{RflFile, RflQuestion};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Question labels to generate frequencies for
    questions: Vec<String>,

    #[arg(short = 'l', long = "layoutfile", help = "The layout file to use, ie p0001.rfl")]
    layout_file: Option<String>,

    #[arg(short = 'd', long = "datafile", help = "The data file to use, ie p0001.fin")]
    data_file: Option<String>,

    #[arg(short = 'v', long = "verbose", help = "Verbose output")]
    _verbose: bool,
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

struct FrequencyStats {
    punch_counts: HashMap<String, usize>,
    valid_cases: usize,
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_precision_loss)]
fn calculate_percentage(count: usize, total: usize) -> u32 {
    if total == 0 {
        0
    } else {
        (count as f64 / total as f64).mul_add(100.0, 0.5).floor() as u32
    }
}

fn print_question_frequency(
    question: &RflQuestion,
    stats: &FrequencyStats,
    total_lines: usize,
) {
    println!("{}:", question.label.to_uppercase());
    
    let main_text = question.main_text();
    if !main_text.is_empty() {
        println!("{}", main_text.join(" "));
    }

    let mut total_responses = 0;

    // Sort response codes for consistent output
    let mut sorted_punches: Vec<_> = question.response_codes.iter().collect();
    sorted_punches.sort_by_key(|(code, _)| code.parse::<i32>().unwrap_or(999));

    for (punch_code, response_text) in sorted_punches {
        let count = stats.punch_counts.get(punch_code).unwrap_or(&0);
        let percentage = calculate_percentage(*count, total_lines);
        
        println!("{punch_code:>4} {response_text:<64} {count:>5} {percentage:>3}%");
        
        total_responses += count;
    }

    println!();

    // Print summary statistics
    let total_pct = calculate_percentage(total_responses, total_lines);
    println!("     {:<64} {:>5} {:>3}%", "TOTAL RESPONSES", total_responses, total_pct);

    let valid_pct = calculate_percentage(stats.valid_cases, total_lines);
    println!("     {:<64} {:>5} {:>3}%", "VALID CASES", stats.valid_cases, valid_pct);

    let missing_cases = total_lines - stats.valid_cases;
    let missing_pct = calculate_percentage(missing_cases, total_lines);
    println!("     {:<64} {:>5} {:>3}%", "MISSING CASES", missing_cases, missing_pct);

    println!("     {:<64} {:>5} {:>3}%", "TOTAL CASES", total_lines, 100);
    println!();
}

fn main() -> IoResult<()> {
    let args = Args::parse();

    // Determine layout file
    let layout_filename = args.layout_file.map_or_else(|| find_file_with_extension(".", &[".rfl"])
            .unwrap_or_else(|| {
                eprintln!("Could not find rfl file in the current directory");
                std::process::exit(1);
            }), |file| file);

    // Determine data file
    let data_filename = args.data_file.map_or_else(|| find_file_with_extension(".", &[".fin"])
                .or_else(|| find_file_with_extension(".", &[".rft"]))
                .unwrap_or_else(|| {
                    eprintln!("Could not find fin or rft file in the current directory");
                    std::process::exit(1);
                }), |file| file);

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
                punch_counts.insert(punch_code.clone(), 0);
            }
            all_stats.insert(question_label.clone(), FrequencyStats {
                punch_counts,
                valid_cases: 0,
            });
        }
    }

    // Process data file once
    let data_file = File::open(&data_filename)?;
    let reader = BufReader::new(data_file);
    let mut line_count = 0;

    for line in reader.lines() {
        let line = line?;
        
        // Process each question for this line
        for question_label in &questions_to_process {
            if let Some(question) = questions_map.get(question_label) {
                let responses = question.responses(&line);
                let mut has_response = false;

                for response in &responses {
                    let response = response.trim();
                    if !response.is_empty() {
                        if let Some(stats) = all_stats.get_mut(question_label) {
                            *stats.punch_counts.entry(response.to_string()).or_insert(0) += 1;
                            has_response = true;
                        }
                    }
                }

                if has_response {
                    if let Some(stats) = all_stats.get_mut(question_label) {
                        stats.valid_cases += 1;
                    }
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
        if let Some(question) = questions_map.get(question_label) {
            if let Some(stats) = all_stats.get(question_label) {
                print_question_frequency(question, stats, line_count);
            }
        }
    }

    Ok(())
}