use std::fs::File;
use std::io::{BufRead, BufReader, Result as IoResult};
use std::path::Path;

use ahash::AHashMap;
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
        help = "The data file to use, eg p0001.fin"
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
    punch_counts: AHashMap<String, f64>,
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

    for (punch_code, response_opt) in sorted_punches {
        let count = stats.punch_counts.get(punch_code).unwrap_or(&0.0);
        let percentage = calculate_percentage(*count, total_weight);
        if let Some(response_text) = response_opt.as_deref() {
            println!("{punch_code:>4} {response_text:<64} {count:>5.0} {percentage:>3}%");
        } else {
            println!("{:>4} {punch_code:<64} {count:>5.0} {percentage:>3}%", "");
        }


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

fn determine_files(args: &Args) -> (String, String) {
    let layout_filename = args.layout_file.clone().map_or_else(
        || {
            find_file_with_extension(".", &[".rfl"]).unwrap_or_else(|| {
                eprintln!("Could not find rfl file in the current directory");
                std::process::exit(1);
            })
        },
        |file| file,
    );

    let data_filename = args.data_file.clone().map_or_else(
        || {
            find_file_with_extension(".", &[".fin"])
                .or_else(|| find_file_with_extension(".", &[".rft"]))
                .or_else(|| find_file_with_extension(".", &[".c"]))
                .unwrap_or_else(|| {
                    eprintln!("Could not find fin or rft file in the current directory");
                    std::process::exit(1);
                })
        },
        |file| file,
    );

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

    if args.questions.is_empty() {
        for question in rfl_file.questions_array() {
            questions_to_process.push(question.label.clone());
        }
    } else {
        for question_label_arg in &args.questions {
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
                            eprintln!("Invalid location format for {question_label_arg}, expected NAME:START.WIDTH");
                            std::process::exit(1);
                        }
                    } else {
                        eprintln!("Invalid location format for {question_label_arg}, expected NAME:START.WIDTH");
                        std::process::exit(1);
                    }
                } else {
                    eprintln!("Could not find question label {question_label_arg} in RFL and format is not NAME:START.WIDTH");
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

fn process_data_file(
    data_filename: &str,
    weight_spec: Option<&WeightSpec>,
    questions_to_process: &[String],
    questions_map: &mut AHashMap<String, RflQuestion>,
    all_stats: &mut AHashMap<String, FrequencyStats>,
) -> IoResult<(usize, f64)> {
    let data_file = File::open(data_filename)?;
    let reader = BufReader::new(data_file);
    let mut line_count = 0;
    let mut total_weight = 0.0;

    for line in reader.lines() {
        let line = line?;

        let weight = weight_spec.map_or(1.0, |spec| spec.extract_weight(&line));
        total_weight += weight;

        for question_label in questions_to_process {
            if let Some(question) = questions_map.get_mut(question_label) {
                let responses = question.extract_responses(&line);
                let mut has_response = false;

                for response in &responses {
                    let response = response.trim();
                    if !response.is_empty() {
                        if !question.responses.contains_key(response) {
                            question.responses.insert(response.to_string(), None);
                        }

                        if let Some(stats) = all_stats.get_mut(question_label) {
                            *stats
                                .punch_counts
                                .entry(response.to_string())
                                .or_insert(0.0) += weight;
                            has_response = true;
                        }
                    }
                }
                if has_response
                    && let Some(stats) = all_stats.get_mut(question_label) {
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

    Ok((line_count, total_weight))
}

fn main() -> IoResult<()> {
    let args = Args::parse();

    let (layout_filename, data_filename) = determine_files(&args);
    let weight_spec = parse_weight_spec(args.weight.as_ref());

    let rfl_file = RflFile::from_file(&layout_filename)?;
    let mut questions_map = rfl_file.questions().clone();

    let questions_to_process = determine_questions_to_process(&args, &rfl_file, &mut questions_map);
    let mut all_stats = initialize_stats(&questions_to_process, &questions_map);

    let (line_count, total_weight) = process_data_file(
        &data_filename,
        weight_spec.as_ref(),
        &questions_to_process,
        &mut questions_map,
        &mut all_stats,
    )?;

    for question_label in &questions_to_process {
        if let Some(question) = questions_map.get(question_label)
            && let Some(stats) = all_stats.get(question_label)
        {
            print_question_frequency(question, stats, line_count, total_weight);
        }
    }

    Ok(())
}
