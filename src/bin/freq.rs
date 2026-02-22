use std::fs::File;
use std::io::{BufRead, BufReader, Result as IoResult};
use std::path::Path;

use ahash::AHashMap;
use clap::Parser;
use dp_library::{CfmcLogic, QuestionType, RflFile, RflQuestion};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Data fields to generate frequencies for
    data_fields: Vec<String>,

    #[arg(
        short = 'l',
        long = "layoutfile",
        help = "The layout file to use, e.g., p0042.rfl"
    )]
    layout_file: Option<String>,

    #[arg(
        short = 'd',
        long = "datafile",
        help = "The data file to use, e.g., P0042.FIN"
    )]
    data_file: Option<String>,

    #[arg(
        short = 'v',
        long = "verbose",
        help = "Full response label shown (no longer truncated)"
    )]
    verbose: bool,

    #[arg(
        short = 'f',
        long = "filter",
        help = "Filter cases, e.g., \"QD7B(02) AND AGEGROUP(1-3)\""
    )]
    logic: Option<String>,

    #[arg(
        short = 'w',
        long = "weight",
        help = "Weight data, e.g., 1000.7 for column 1000, width 7"
    )]
    weight_loc: Option<String>,
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
        let start_idx = self.start_col - 1;
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
    punch_counts: AHashMap<String, f64>,
    valid_cases: f64,
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
fn calculate_percentage(count: f64, total: f64) -> f64 {
    if total == 0.0 {
        0.0
    } else {
        // (count / total).mul_add(100.0, 0.5).floor() as u32
        (count / total) * 100.
    }
}

struct ColumnFormatter {
    text_width: usize,
    truncate: bool,
}

impl ColumnFormatter {
    fn new(verbose: bool, max_text_len: usize) -> Self {
        if verbose {
            Self {
                text_width: max_text_len.max(64),
                truncate: false,
            }
        } else {
            Self {
                text_width: 64,
                truncate: true,
            }
        }
    }

    fn format_row(&self, code: &str, text: &str, count: f64, pct: f64) -> String {
        let w = self.text_width;
        let display = if self.truncate && text.chars().count() > w {
            let t: String = text.chars().take(w - 3).collect();
            format!("{t}...")
        } else {
            text.to_string()
        };
        format!("{code:>4} {display:<w$} {count:>6.0} {pct:>6.1}%")
    }
}

fn print_question_frequency(
    verbose: bool,
    question: &RflQuestion,
    stats: &FrequencyStats,
    _total_lines: f64,
    total_weight: f64,
) {
    println!("{}:", question.label.to_uppercase());

    let main_text = question.main_text();
    if !main_text.is_empty() {
        println!("{}\n", main_text.join("\n\n"));
    }

    let mut total_responses = 0.0;

    // Sort response codes for consistent output
    let mut sorted_punches: Vec<_> = question.responses.iter().collect();
    sorted_punches.sort_by(|(a_code, _), (b_code, _)| {
        let a_num = a_code.parse::<i32>();
        let b_num = b_code.parse::<i32>();
        match (a_num, b_num) {
            (Ok(a), Ok(b)) => a.cmp(&b),
            (Err(_), Err(_)) => a_code.cmp(b_code),
            (Ok(_), Err(_)) => std::cmp::Ordering::Less,
            (Err(_), Ok(_)) => std::cmp::Ordering::Greater,
        }
    });

    let max_text_len = sorted_punches
        .iter()
        .map(|(code, resp)| resp.as_ref().map_or(code.len(), String::len))
        .max()
        .unwrap_or(0);
    let fmt = ColumnFormatter::new(verbose, max_text_len);

    for (punch_code, response_opt) in sorted_punches {
        let count = *stats.punch_counts.get(punch_code).unwrap_or(&0.0);
        let pct = calculate_percentage(count, total_weight);
        let (code, text) = response_opt
            .as_deref()
            .map_or(("", punch_code.as_str()), |t| (punch_code.as_str(), t));
        println!("{}", fmt.format_row(code, text, count, pct));
        total_responses += count;
    }

    println!();

    let total_pct = calculate_percentage(total_responses, total_weight);
    println!(
        "{}",
        fmt.format_row("", "TOTAL RESPONSES", total_responses, total_pct)
    );

    let valid_pct = calculate_percentage(stats.valid_cases, total_weight);
    println!(
        "{}",
        fmt.format_row("", "VALID CASES", stats.valid_cases, valid_pct)
    );

    let missing_weight = total_weight - stats.valid_cases;
    let missing_pct = calculate_percentage(missing_weight, total_weight);
    println!(
        "{}",
        fmt.format_row("", "MISSING CASES", missing_weight, missing_pct)
    );

    println!("{}", fmt.format_row("", "TOTAL CASES", total_weight, 100.0));
    println!();
}

fn print_filter_frequencies(total_lines: f64, lines_processed: f64) {
    let lines_excluded = total_lines - lines_processed;
    let included_pct = calculate_percentage(lines_processed, total_lines);
    let excluded_pct = calculate_percentage(lines_excluded, total_lines);

    println!("Total lines:           {total_lines:>6.0}");
    println!("Lines matching filter: {lines_processed:>6.0} {included_pct:>6.1}%");
    println!("Lines excluded:        {lines_excluded:>6.0} {excluded_pct:>6.1}%");
    println!();
}

fn determine_files(args: &Args) -> (String, String) {
    let layout_filename = args.layout_file.clone().unwrap_or_else(|| {
        find_file_with_extension(".", &[".rfl"]).unwrap_or_else(|| {
            eprintln!("Could not find rfl file in the current directory");
            std::process::exit(1);
        })
    });

    let data_filename = args.data_file.clone().unwrap_or_else(|| {
        find_file_with_extension(".", &[".c"])
            .or_else(|| find_file_with_extension(".", &[".fin"]))
            .or_else(|| find_file_with_extension(".", &[".rft"]))
            .unwrap_or_else(|| {
                eprintln!("Could not find fin or rft file in the current directory");
                std::process::exit(1);
            })
    });

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

    (layout_filename, data_filename)
}

#[allow(clippy::single_option_map)]
fn parse_weight_spec(weight_arg: Option<&String>) -> Option<WeightSpec> {
    weight_arg.map(|weight_str| match WeightSpec::parse(weight_str) {
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
    })
}

fn determine_questions_to_process(
    args: &Args,
    rfl_file: &RflFile,
    questions_map: &mut AHashMap<String, RflQuestion>,
) -> Vec<String> {
    let mut questions_to_process = Vec::new();

    if args.data_fields.is_empty() {
        for question in rfl_file.questions_array() {
            questions_to_process.push(question.label.clone());
        }
    } else {
        for question_label_arg in &args.data_fields {
            let uc_question_label = question_label_arg.to_uppercase();
            let mut question_label = uc_question_label.clone();
            if !questions_map.contains_key(&question_label) {
                let parts: Vec<&str> = uc_question_label.split(':').collect();
                if parts.len() == 2 {
                    let name = parts[0].to_string();
                    question_label.clone_from(&name);
                    let location: Vec<&str> = parts[1].split('.').collect();
                    if location.len() == 2 {
                        if let (Ok(start), Ok(width)) = (location[0].parse(), location[1].parse()) {
                            let new_question = RflQuestion::new_raw(name, start, width);
                            questions_map.insert(question_label.clone(), new_question);
                        } else {
                            eprintln!(
                                "Invalid location format for {question_label_arg}, expected NAME:START.WIDTH"
                            );
                            std::process::exit(1);
                        }
                    } else {
                        eprintln!(
                            "Invalid location format for {question_label_arg}, expected NAME:START.WIDTH"
                        );
                        std::process::exit(1);
                    }
                } else {
                    eprintln!(
                        "Could not find question label {question_label_arg} in RFL and format is not NAME:START.WIDTH"
                    );
                    std::process::exit(1);
                }
            }
            questions_to_process.push(question_label);
        }
    }

    questions_to_process
}

fn initialize_stats(
    questions_to_process: &[String],
    questions_map: &AHashMap<String, RflQuestion>,
) -> AHashMap<String, FrequencyStats> {
    let mut all_stats: AHashMap<String, FrequencyStats> = AHashMap::new();

    for question_label in questions_to_process {
        if let Some(question) = questions_map.get(question_label) {
            let mut punch_counts = AHashMap::new();
            for punch_code in question.responses.keys() {
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

    all_stats
}

fn parse_and_validate_filter(filter_str: &str) -> CfmcLogic {
    match CfmcLogic::parse(filter_str) {
        Ok(logic) => {
            println!("Using filter: {filter_str}");
            logic
        }
        Err(e) => {
            eprintln!("Error parsing filter expression: {e}");
            eprintln!("Filter: {filter_str}");
            std::process::exit(1);
        }
    }
}

fn process_data_file(
    data_filename: &str,
    weight_spec: Option<&WeightSpec>,
    questions_to_process: &[String],
    questions_map: &mut AHashMap<String, RflQuestion>,
    all_stats: &mut AHashMap<String, FrequencyStats>,
    filter: Option<&CfmcLogic>,
) -> IoResult<(f64, f64, f64)> {
    let data_file = File::open(data_filename)?;
    let reader = BufReader::new(data_file);
    let mut total_lines = 0.0;
    let mut lines_processed = 0.0;
    let mut total_weight = 0.0;

    for line in reader.lines() {
        let line = line?;
        let weight = weight_spec.map_or(1.0, |spec| spec.extract_weight(&line));
        total_lines += weight;

        if let Some(filter_logic) = filter {
            match filter_logic.evaluate(questions_map, &line) {
                Ok(matched) => {
                    if !matched {
                        continue;
                    }
                }
                Err(e) => {
                    eprintln!("Error evaluating filter on line {total_lines}: {e}");
                    std::process::exit(1);
                }
            }
        }

        total_weight += weight;

        for question_label in questions_to_process {
            if let Some(question) = questions_map.get_mut(question_label) {
                let responses = question.extract_responses(&line);
                let mut has_response = false;

                for response in &responses {
                    let mut response = response.trim().to_string();
                    if !response.is_empty() {
                        if !question.responses.contains_key(&response) {
                            if response.parse::<u32>().is_ok() && question.question_type == QuestionType::Fld {
                                let pad = question.width;
                                response = format!("{response:0>pad$}");
                                if !question.responses.contains_key(&response) {
                                    question.responses.insert(response.clone(), None);
                                }
                            } else {
                                question.responses.insert(response.clone(), None);
                            }
                        }

                        if let Some(stats) = all_stats.get_mut(question_label) {
                            *stats
                                .punch_counts
                                .entry(response.clone())
                                .or_insert(0.0) += weight;
                            has_response = true;
                        }
                    }
                }

                if has_response && let Some(stats) = all_stats.get_mut(question_label) {
                    stats.valid_cases += weight;
                }
            }
        }

        lines_processed += weight;
        if lines_processed % 100.0 == 0.0 {
            print!(".");
            std::io::Write::flush(&mut std::io::stdout()).unwrap();
        }
    }
    println!();

    Ok((total_lines, lines_processed, total_weight))
}

fn main() -> IoResult<()> {
    let args = Args::parse();

    let (layout_filename, data_filename) = determine_files(&args);
    let weight_spec = parse_weight_spec(args.weight_loc.as_ref());

    let rfl_file = RflFile::from_file(&layout_filename)?;
    let mut questions_map = rfl_file.questions().clone();

    let questions_to_process = determine_questions_to_process(&args, &rfl_file, &mut questions_map);

    let filter = args
        .logic
        .as_ref()
        .map(|filter_str| parse_and_validate_filter(filter_str));

    let mut all_stats = initialize_stats(&questions_to_process, &questions_map);

    let (total_lines, lines_processed, total_weight) = process_data_file(
        &data_filename,
        weight_spec.as_ref(),
        &questions_to_process,
        &mut questions_map,
        &mut all_stats,
        filter.as_ref(),
    )?;

    // Report filtering statistics if a filter was used
    #[allow(clippy::cast_precision_loss)]
    if filter.is_some() {
        print_filter_frequencies(total_lines, lines_processed);
    }

    for question_label in &questions_to_process {
        if let Some(question) = questions_map.get(question_label)
            && let Some(stats) = all_stats.get(question_label)
        {
            print_question_frequency(args.verbose, question, stats, lines_processed, total_weight);
        }
    }

    Ok(())
}
