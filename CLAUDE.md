# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

### Build and Test
```bash
cargo build          # Build the library
cargo test           # Run all tests
cargo test --lib     # Run library tests only
cargo test test_name # Run a specific test
cargo clippy         # Run linter
cargo fmt            # Format code
```

### Build Binaries
```bash
cargo build --bin chkcounts  # Build chkcounts binary
cargo build --bin freq       # Build freq binary
cargo build                  # Build all targets
```

### Development Workflow
```bash
cargo check          # Fast compilation check
cargo doc --open     # Generate and view documentation
```

## Architecture

This is a Rust library for processing survey data formats, originally ported from Perl survey processing tools.

### Core Modules

**`src/rfl.rs`** - RFL (Record Format Layout) parsing
- `RflFile`: Main parser for .rfl files containing survey question metadata
- `RflQuestion`: Individual question definition with position, type, and response codes
- `QuestionType`: Enum for FLD (categorical), VAR (variable), NUM (numeric)
- Parses fixed-width survey data format specifications

**`src/cfmc.rs`** - CFMC logic expression parsing and evaluation  
- `CfmcLogic`: Parser and evaluator for survey skip logic expressions
- `CfmcNode`: AST nodes for logic expressions (Binary/Unary operations, literals, question references)
- `CfmcOperator`: Comprehensive set of logical, comparison, and special survey operators
- Supports complex expressions like `COMP(1) AND (QB(01-56) OR AGEGROUP(1,2,3))`

### Data Processing Flow
1. Parse .rfl files to extract question metadata and data layout
2. Parse CFMC logic expressions for survey branching/skip logic  
3. Evaluate logic expressions against actual survey response data
4. Extract responses from fixed-width data lines using question positioning

### Key Design Patterns
- Uses `HashMap<String, RflQuestion>` for fast question lookups by label
- AST-based expression parsing with operator precedence handling
- Comprehensive test coverage with both unit and integration tests
- Error handling with descriptive messages for parsing failures

## Binary Applications

The project includes two command-line tools ported from Perl:

**`chkcounts`** - Counter validation tool
- Validates CFMC logic expressions against survey data
- Compares logic match counts with actual response counts  
- Supports verbose mode to show mismatches
- Usage: `./target/debug/chkcounts [-l layout.rfl] [-d data.fin] [-c counters.chk] [-v]`

**`freq`** - Frequency analysis tool  
- Generates frequency tables for survey questions
- Shows response counts, percentages, and case statistics
- Can process single questions, multiple questions, or all questions
- Usage: `./target/debug/freq [QUESTION1 QUESTION2...] [-l layout.rfl] [-d data.fin]`

### Dependencies
- `regex = "1.0"` - Used for text pattern matching and cleaning question prefixes
- `clap = "4.0"` - Command-line argument parsing for binaries

### File Formats
- `.rfl` files: Survey question layout definitions with Q/T/R/X line types
- Response data: Fixed-width text lines with responses at specified column positions (`.fin`, `.rft`)
- CFMC expressions: Logical expressions referencing question labels for branching logic
- `.chk` files: Counter definitions in format `LABEL: LOGIC_EXPRESSION`

### Important Notes
- RFL files may contain relocated data positions in format `[original] --> [actual]` 
- The parser uses the actual position (after `-->`) for data extraction
- Four-digit study numbers are used as file naming convention (e.g., `p0157.rfl` for study 0157)