use clap::Parser;
use dp_library::{BannersTables, RflFile};
use std::fs;
use std::path::Path;
use std::process;

#[derive(Parser)]
#[command(name = "banners")]
#[command(about = "Generate banner crosstab specifications from Excel files")]
#[command(version = "1.0")]
struct Args {
    /// Excel file containing banner specifications (.xlsx)
    excel_file: Option<String>,
    
    /// RFL layout file (.rfl)
    #[arg(short = 'l', long)]
    layout: Option<String>,
    
    /// Output text file for banners (.txt)
    #[arg(short = 'o', long)]
    output: Option<String>,
    
    /// Footer type (nbc, nmb, nbs, r2r, pos)
    #[arg(short = 'f', long, default_value = "pos")]
    footer: String,
    
    /// Files to process (excel, rfl, txt, footer files)
    files: Vec<String>,
}

fn main() {
    let args = Args::parse();
    
    // Parse file arguments based on extensions
    let mut excel_file = args.excel_file;
    let mut rfl_file = args.layout;
    let mut output_file = args.output;
    let mut footer_type = args.footer;
    
    // Process additional files by extension
    for file in &args.files {
        let path = Path::new(file);
        if let Some(ext) = path.extension() {
            match ext.to_str().unwrap_or("").to_lowercase().as_str() {
                "xlsx" | "xls" => {
                    if excel_file.is_none() {
                        excel_file = Some(file.clone());
                    }
                }
                "rfl" => {
                    if rfl_file.is_none() {
                        rfl_file = Some(file.clone());
                    }
                }
                "txt" => {
                    if output_file.is_none() {
                        output_file = Some(file.clone());
                    }
                }
                _ => {
                    // Check for footer type patterns
                    let filename = file.to_lowercase();
                    if filename.contains("nbc") {
                        footer_type = "nbc".to_string();
                    } else if filename.contains("nmb") {
                        footer_type = "nmb".to_string();
                    } else if filename.contains("nbs") {
                        footer_type = "nbs".to_string();
                    } else if filename.contains("r2r") {
                        footer_type = "r2r".to_string();
                    } else if filename.contains("pos") {
                        footer_type = "pos".to_string();
                    } else if file != "-" {
                        eprintln!("Warning: Ignoring {} because it does not have a proper extension.", file);
                    }
                }
            }
        }
    }
    
    // Validate required files
    let excel_file = excel_file.unwrap_or_else(|| {
        eprintln!("Error: Excel input file not specified");
        process::exit(1);
    });
    
    let rfl_file = rfl_file.unwrap_or_else(|| {
        eprintln!("Error: RFL file not specified");
        process::exit(1);
    });
    
    let output_file = output_file.unwrap_or_else(|| {
        // Default output file name based on excel file
        let excel_path = Path::new(&excel_file);
        excel_path.with_extension("txt").to_string_lossy().to_string()
    });
    
    // Check if files exist
    if !Path::new(&excel_file).exists() {
        eprintln!("Error: {} does not exist", excel_file);
        process::exit(1);
    }
    
    if !Path::new(&rfl_file).exists() {
        eprintln!("Error: {} does not exist", rfl_file);
        process::exit(1);
    }
    
    // Load RFL file
    println!("Loading RFL file: {}", rfl_file);
    let rfl = match RflFile::from_file(&rfl_file) {
        Ok(rfl) => rfl,
        Err(e) => {
            eprintln!("Error loading RFL file: {}", e);
            process::exit(1);
        }
    };
    let questions = rfl.questions();
    println!("{} loaded", rfl_file);
    
    // Load Excel file
    println!("Loading Excel file: {}", excel_file);
    let banners_tables = match BannersTables::from_excel(&excel_file) {
        Ok(tables) => tables,
        Err(e) => {
            eprintln!("Error loading Excel file: {}", e);
            process::exit(1);
        }
    };
    println!("{} loaded", excel_file);
    
    // Generate banner output
    let banner_content = match banners_tables.generate_banner_output(&questions, &footer_type) {
        Ok(content) => content,
        Err(e) => {
            eprintln!("Error generating banner output: {}", e);
            process::exit(1);
        }
    };
    
    // Write output file
    if let Err(e) = fs::write(&output_file, banner_content) {
        eprintln!("Error writing output file {}: {}", output_file, e);
        process::exit(1);
    }
    
    // Set permissions (equivalent to chmod 666)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = fs::set_permissions(&output_file, fs::Permissions::from_mode(0o666)) {
            eprintln!("Warning: Could not set permissions on {}: {}", output_file, e);
        }
    }
    
    println!("Banner output written to: {}", output_file);
}