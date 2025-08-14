use clap::Parser;
use dp_library::{BannersTables, RflFile};
use std::fs;
use std::path::Path;
use std::process;

#[derive(Parser)]
#[command(name = "banners")]
#[command(about = "Generate crosstab banners from Excel specification files")]
#[command(version = "1.0")]
struct Cli {
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

    /// Arguments to process (excel, rfl, txt, footer)
    args: Vec<String>,
}

fn parse_files(cli: &Cli) -> (String, String, String, String) {
    let mut excel_file = cli.excel_file.clone();
    let mut rfl_file = cli.layout.clone();
    let mut output_file = cli.output.clone();
    let mut footer_type = cli.footer.clone();

    // Process additional files by extension
    for arg in &cli.args {
        let path = Path::new(arg);
        if let Some(ext) = path.extension() {
            match ext.to_str().unwrap_or("").to_lowercase().as_str() {
                "xlsx" | "xls" => {
                    if excel_file.is_none() {
                        excel_file = Some(arg.clone());
                    }
                }
                "rfl" => {
                    if rfl_file.is_none() {
                        rfl_file = Some(arg.clone());
                    }
                }
                "txt" | "e" => {
                    if output_file.is_none() {
                        output_file = Some(arg.clone());
                    }
                }
                _ => (),
            }
        } else {
            // Check for footer type patterns
            footer_type = match arg.as_str() {
                "nbc" => "nbc".to_string(),
                "nmb" => "nmb".to_string(),
                "nbs" => "nbs".to_string(),
                "r2r" => "r2r".to_string(),
                _ => "pos".to_string(),
            }
        }
    }

    let excel_file = excel_file.unwrap_or_else(|| {
        eprintln!("Error: Excel input file not specified");
        process::exit(1);
    });

    let rfl_file = rfl_file.unwrap_or_else(|| {
        eprintln!("Error: RFL file not specified");
        process::exit(1);
    });

    let output_file = output_file.unwrap_or_else(|| {
        let excel_path = Path::new(&excel_file);
        excel_path
            .with_extension("txt")
            .to_string_lossy()
            .to_string()
    });

    (excel_file, rfl_file, output_file, footer_type)
}

fn validate_files_exist(excel_file: &str, rfl_file: &str) {
    if !Path::new(excel_file).exists() {
        eprintln!("Error: {excel_file} does not exist");
        process::exit(1);
    }

    if !Path::new(rfl_file).exists() {
        eprintln!("Error: {rfl_file} does not exist");
        process::exit(1);
    }
}

fn write_output_with_permissions(output_file: &str, content: String) {
    if let Err(e) = fs::write(output_file, content) {
        eprintln!("Error writing output file {output_file}: {e}");
        process::exit(1);
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = fs::set_permissions(output_file, fs::Permissions::from_mode(0o666)) {
            eprintln!("Warning: Could not set permissions on {output_file}: {e}");
        }
    }
}

fn main() {
    let args = Cli::parse();
    let (excel_file, rfl_file, output_file, footer_type) = parse_files(&args);

    validate_files_exist(&excel_file, &rfl_file);

    // Load RFL file
    let rfl = match RflFile::from_file(&rfl_file) {
        Ok(rfl) => rfl,
        Err(e) => {
            eprintln!("Error loading RFL file: {e}");
            process::exit(1);
        }
    };
    let questions = rfl.questions();
    println!("{rfl_file} loaded");

    // Load Excel file
    let banners_tables = match BannersTables::from_excel(&excel_file) {
        Ok(tables) => tables,
        Err(e) => {
            eprintln!("Error loading Excel file: {e}");
            process::exit(1);
        }
    };
    println!("{excel_file} loaded");

    // Generate banner output
    let banner_content = match banners_tables.generate_banner_output(questions, &footer_type) {
        Ok(content) => content,
        Err(e) => {
            eprintln!("Error generating banner output: {e}");
            process::exit(1);
        }
    };

    write_output_with_permissions(&output_file, banner_content);
    println!("Banner output written to: {output_file}");
}
