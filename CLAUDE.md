# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

This is a Rust library (`dp_library`) for survey data processing, originally ported from Perl tools. The library provides three main functionalities:

1. **RFL (Record Format Layout) parsing** - Parses survey question metadata from .rfl files
2. **CFMC logic expression parsing** - Evaluates survey skip logic expressions 
3. **Cross-tabulation banner generation** - Creates banners from Excel specifications

## Commands

### Building and Testing
```bash
cargo build          # Build the library and binaries
cargo check          # Type check without building
cargo test            # Run all tests
cargo nag             # Stricter linting (alias for cargo clippy)
cargo fixnags         # Auto-fix linting issues where possible
```

### Running Binaries
The project provides three binary tools:
```bash
cargo run --bin freq -- [questions] -l layout.rfl -d data.fin    # Generate frequency tables
cargo run --bin chkcounts -- -l layout.rfl -d data.fin -c counters.chk    # Check counter logic
cargo run --bin banners -- excel_file.xlsx -l layout.rfl -o output.txt    # Generate crosstab banners
```

## Architecture

### Core Library Structure (`src/lib.rs`)
- **`rfl` module**: Handles RFL file parsing, question metadata, and data types
- **`cfmc` module**: CFMC logic expression parsing and evaluation engine
- **`crosstabs` module**: Banner generation and cross-tabulation logic

### Key Types
- `RflQuestion`: Survey question with metadata (start column, width, response codes, etc.)
- `CfmcLogic`/`CfmcNode`: Logic expression AST for survey skip logic evaluation
- `BannersTables`: Cross-tabulation banner specifications from Excel files

### Binary Tools
- **`freq.rs`**: Frequency table generation with optional weighting support
- **`chkcounts.rs`**: Counter logic validation against survey data
- **`banners.rs`**: Excel-to-banner conversion with configurable footer types

### Data Processing Flow
1. RFL files define survey question structure and metadata
2. CFMC expressions handle conditional logic and skip patterns
3. Data files (.fin) contain actual survey responses
4. Tools process combinations of layout + data + logic files

### Dependencies
- `ahash`: High-performance HashMap replacement used throughout
- `calamine`: Excel file reading for banner specifications
- `clap`: Command-line argument parsing for all binaries
- `regex`: Pattern matching for RFL parsing and text processing