# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

This is a Rust library (`dp_library`) for survey data processing, originally ported from Perl. It provides functionality for parsing and processing survey data formats:

- **RFL (Record Format Layout)** parsing for survey question metadata 
- **CFMC logic expression** parsing and evaluation for survey skip logic
- **CrossTabs** functionality for generating survey crosstabulation reports

The project includes both the Rust implementation and the original Perl code for reference.

## Build and Development Commands

- `cargo build` - Build the library and binaries
- `cargo test` - Run tests  
- `cargo run --bin <name>` - Run specific binary (chkcounts, freq, banners)
- `cargo nag` - Stricter linting (alias for cargo clippy)
- `cargo fixnags` - Auto-fix lint issues where possible

## Architecture

### Core Modules

- **`src/rfl.rs`** - RFL file parsing for survey question definitions. Contains `RflFile`, `RflQuestion`, and `QuestionType` structs
- **`src/cfmc.rs`** - CFMC logic expression parsing and evaluation. Contains `CfmcLogic`, `CfmcNode`, and `CfmcOperator` for survey skip logic
- **`src/crosstabs.rs`** - Crosstab report generation from Excel specifications. Contains `CrossTabsLogic`, `Banner`, and related types

### Binary Tools

- **`chkcounts`** - Validates survey data against counter specifications using RFL layout files and CFMC logic
- **`freq`** - Generates frequency reports for specified survey questions 
- **`banners`** - Generates crosstab banner specifications from Excel files

### Dependencies

- `regex` - Pattern matching for parsing survey data formats
- `clap` - Command-line argument parsing for the binary tools
- `calamine` - Excel file reading for banner specifications  
- `serde` - Serialization support

### Legacy Perl Code

The `perl/` directory contains the original Perl implementation this was ported from:
- `cfmclogicclass.pm` - Original CFMC logic parser
- `rflclass.pm` / `rflquestionclass.pm` - Original RFL parsers
- `crosstabsclass.pm` - Original crosstab functionality

## Data Flow

1. **RFL files** (.rfl) define survey question layouts and metadata
2. **Data files** (.fin) contain survey response data in fixed-width format
3. **CFMC logic expressions** define skip logic and validation rules
4. **Excel files** (.xlsx) specify crosstab banner configurations
5. **Counter files** (.chk) define validation counters for data quality checks

The tools process these inputs to generate frequency reports, crosstab banners, and data validation results.