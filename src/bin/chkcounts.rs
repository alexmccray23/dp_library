use std::fs::File;
use std::io::{BufRead, BufReader, Result as IoResult};
use std::path::Path;

use clap::Parser;
use library::{CfmcLogic, RflFile};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(short = 'l', long = "layoutfile", help = "The layout file to use, ie p0001.rfl")]
    layout_file: Option<String>,

    #[arg(short = 'd', long = "datafile", help = "The data file to use, ie p0001.fin")]
    data_file: Option<String>,

    #[arg(short = 'c', long = "countersfile", help = "The counter file to use, ie counters.chk")]
    counters_file: Option<String>,

    #[arg(short = 'v', long = "verbose", help = "Output mismatches between logic and responses")]
    verbose: bool,
}

struct Counter {
    label: String,
    logic: Option<CfmcLogic>,
    logic_count: usize,
    response_count: usize,
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

fn load_counters(filename: &str) -> IoResult<Vec<Counter>> {
    let file = File::open(filename)?;
    let reader = BufReader::new(file);
    let mut counters = Vec::new();

    for line in reader.lines() {
        let line = line?;
        let line = line.trim();
        
        // Skip comments and empty lines
        if line.starts_with('#') || line.is_empty() {
            continue;
        }

        // Parse label:logic format
        if let Some(colon_pos) = line.find(':') {
            let label = line[..colon_pos].trim().to_string();
            let logic_str = line[colon_pos + 1..].trim();

            if logic_str.is_empty() {
                counters.push (Counter {
                    label,
                    logic: None,
                    logic_count: 0,
                    response_count: 0,
                });
                continue;
            }
            
            match CfmcLogic::parse(logic_str) {
                Ok(logic) => {
                    counters.push(Counter {
                        label,
                        logic: Some(logic),
                        logic_count: 0,
                        response_count: 0,
                    });
                }
                Err(e) => {
                    eprintln!("ERROR: {e} for '{label}: {logic_str}'");
                    // std::process::exit(1);
                }
            }
        }
    }

    Ok(counters)
}

fn extract_case_id(line: &str) -> &str {
    if line.len() >= 9 {
        &line[0..9]
    } else {
        line
    }
}

fn determine_file_paths(args: &Args) -> (String, String, String) {
    // Determine counters file
    let counters_filename = args.counters_file
        .as_ref()
        .map_or_else(|| "counters.chk".to_string(), std::clone::Clone::clone);

    // Determine layout file
    let layout_filename = args.layout_file.as_ref().map_or_else(|| {
        find_file_with_extension(".", &[".rfl"])
            .unwrap_or_else(|| {
                eprintln!("Could not find rfl file in the current directory");
                std::process::exit(1);
            })
    }, std::clone::Clone::clone);

    // Determine data file
    let data_filename = args.data_file.as_ref().map_or_else(|| {
        find_file_with_extension(".", &[".fin"])
            .or_else(|| find_file_with_extension(".", &[".rft"]))
            .unwrap_or_else(|| {
                eprintln!("Could not find fin or rft file in the current directory");
                std::process::exit(1);
            })
    }, std::clone::Clone::clone);

    (counters_filename, layout_filename, data_filename)
}

fn main() -> IoResult<()> {
    let args = Args::parse();

    let (counters_filename, layout_filename, data_filename) = determine_file_paths(&args);

    // Verify files exist
    if !Path::new(&counters_filename).exists() {
        eprintln!("{counters_filename} not found in the current directory");
        std::process::exit(1);
    }
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
    println!("Using counters file {counters_filename}");

    // Load files
    let mut counters = load_counters(&counters_filename)?;
    let rfl_file = RflFile::from_file(&layout_filename)?;
    let questions = rfl_file.questions();

    // Process data file
    let data_file = File::open(&data_filename)?;
    let reader = BufReader::new(data_file);

    let mut line_count = 0;
    for line in reader.lines() {
        let line = line?;
        
        // Process each counter
        for counter in &mut counters {
            // Evaluate logic
            if let Some(ref logic) = counter.logic {
                match logic.evaluate(questions, &line) {
                    Ok(matched) => {
                        if matched {
                            counter.logic_count += 1;
                        }

                        // Count responses if question exists in RFL
                        if let Some(question) = questions.get(&counter.label) {
                            let responses = question.responses(&line);
                            let has_response = responses.iter().any(|r| !r.trim().is_empty());

                            if has_response {
                                counter.response_count += 1;
                            }

                            // Verbose output for mismatches
                            if args.verbose {
                                let case_id = extract_case_id(&line);
                                if !matched && has_response {
                                    println!("Logic did not match for {} but response not blank on case id {}.", 
                                        counter.label, case_id);
                                } else if matched && !has_response {
                                    println!("Matched for {} but response blank on case id {}.", 
                                        counter.label, case_id);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Error in {}: {}", counter.label, e);
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

    // Output results
    println!("Label:                LOGIC LABEL");
    for counter in &counters {
        if questions.contains_key(&counter.label) {
            let output = if counter.logic_count == counter.response_count {
                format!("{:<21}{:6}{:7}", 
                        format!("{}:", counter.label),
                        counter.logic_count,
                        counter.response_count)
            } else {
                format!("{:<21}{:6}{:7}<--", 
                        format!("{}:", counter.label),
                        counter.logic_count,
                        counter.response_count)
            };
            println!("{output}");
        } else {
            println!("{:<21}{:6}", 
                     format!("{}:", counter.label),
                     counter.logic_count);
        }
    }

    Ok(())
}
