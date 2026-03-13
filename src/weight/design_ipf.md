# Weight Module Design Spec

## Overview

The `weight` module within `dp_library` connects survey data (fixed-width
records), question metadata (RFL layouts), and the `ipf_survey` raking engine.
It parses weight table definitions from UNCLE `.E` files with UNCLE qualification
syntax, classifies respondents, runs iterative proportional fitting (raking), and
returns per-record weights.

## Location

All code lives within `dp_library` at `/home/alexm/rust/vd/dp_library/`.

### Directory Structure

```
dp_library/src/
  lib.rs            - pub mod weight;
  cfmc.rs           - CFMC logic parser/evaluator
  rfl.rs            - RFL layout parser
  crosstabs.rs      - crosstab/banner logic
  weight/
    mod.rs          - public re-exports
    ipf.rs          - glue layer: condition evaluation, CodedSurvey
                      construction, raking orchestration
    uncle.rs        - UNCLE qualification syntax parser and evaluator
    parse_e.rs      - .E file parser: extracts weight table definitions,
                      X WEIGHT directives, X SET QUAL qualifiers, X IF
                      column assignments
  bin/
    weight.rs       - CLI binary orchestrating the full pipeline
```

### Cargo.toml

```toml
[dependencies]
ipf_survey = { path = "../ipf_survey" }
```

```toml
[[bin]]
name = "weight"
path = "src/bin/weight.rs"
```

## Inputs

### 1. Weight Table Definitions

A weight scheme is a sequence of **weight tables**, each representing one raking
dimension (one marginal constraint). Each table contains **category rows** that
partition the respondent population.

```rust
pub struct WeightScheme {
    pub tables: Vec<WeightTable>,
    pub config: WeightConfig,
}

pub struct WeightTable {
    pub id: u16,
    pub label: Option<String>,
    pub categories: Vec<WeightCategory>,
}

pub struct WeightCategory {
    pub label: String,
    pub condition: WeightCondition,
    pub target: f64,
}
```

### 2. Conditions

Conditions use the `WeightCondition` dispatch enum, supporting both UNCLE
position-based syntax (the default for production `.E` files) and CFMC
label-based syntax (for programmatic use with RFL metadata):

```rust
#[derive(Clone)]
pub enum WeightCondition {
    Cfmc(CfmcLogic),
    Uncle(UncleExpr),
}

impl WeightCondition {
    pub fn evaluate(
        &self,
        questions: Option<&AHashMap<String, RflQuestion>>,
        record: &str,
    ) -> Result<bool, String> {
        match self {
            Self::Cfmc(logic) => {
                let questions = questions
                    .ok_or("RFL layout file is required to evaluate CFMC conditions")?;
                logic.evaluate(questions, record)
            }
            Self::Uncle(expr) => expr.evaluate(record),
        }
    }
}
```

UNCLE is self-contained (evaluates directly against fixed-width column
positions). CFMC requires an RFL file to resolve question labels to column
positions. The `.E` file parser always produces `WeightCondition::Uncle`.

### Supported UNCLE Syntax

| Syntax                        | Example                              |
|-------------------------------|--------------------------------------|
| Position/code test            | `1!43-1`                             |
| Code range                    | `1!43-2:3`                           |
| Code list                     | `1!194-8,9`                          |
| Inline NOT                    | `1!49N1`                             |
| Blank test                    | `1!43-$`                             |
| Position list/range           | `1!65,67-1`, `1!65:67-1`            |
| NOT prefix                    | `NOT(1!44-1)`                        |
| AND (explicit/implicit)       | `1!48-6 AND 1!51-1`, `1!48-6 1!51-1`|
| OR                            | `1!35-1 OR 1!36-2`                   |
| R() numeric range qualifier   | `R(1!26:27,9,23,25,33,44,50)`       |
| Literal character match       | `1!1949 'R'`                         |

### Supported CFMC Syntax

| Syntax                        | Example                              |
|-------------------------------|--------------------------------------|
| CFMC expression               | `GENDER(1)`                          |
| CFMC with code range          | `GENDER(2,3)` / `AGEGROUP(1-3)`     |
| CFMC AND/OR/NOT               | `QD3A(6) AND VERSION(1)`            |
| CFMC NOT                      | `NOT QE(1)`                          |
| Literal character match       | `[GEOCODE+0.1$]="R"`                 |

### 3. Survey Data

- An optional `RflFile` for question metadata (required only when CFMC
  conditions or base weight fields are used).
- A `&[impl AsRef<str>]` of fixed-width data lines, one per respondent.

### 4. Configuration

```rust
pub struct WeightConfig {
    /// Passed through to ipf_survey::rake()
    pub raking: RakingConfig,

    /// Optional base weight field -- an RFL question label whose numeric value
    /// is the respondent's design weight. If None, all base weights are 1.0.
    pub base_weight_field: Option<String>,

    /// Raw column range (1-based, inclusive) containing a prior weight to use
    /// as the base weight. Parsed from CWEIGHT + RETAIN in .E files. Takes
    /// precedence over base_weight_field when both are set.
    pub base_weight_columns: Option<(usize, usize)>,

    /// Tolerance for grand-total consistency across weight tables.
    /// Real-world tables often have rounded targets that don't sum identically.
    /// Default: 1e-6. Set higher (e.g. 0.005) for typical production tables.
    pub target_tolerance: Option<f64>,
}
```

## Processing Pipeline

```
                   +----------------+
                   | WeightScheme   |  (tables + config)
                   +-------+--------+
                           |
                           v
              +------------------------+
              |  Classify Records      |  For each respondent, evaluate each
              |                        |  table's conditions -> category index
              +------------------------+
                           |
                           v
              +------------------------+
              |  Build CodedSurvey     |  N records x K tables matrix
              |  + PopulationTargets   |  One Variable per table
              +------------------------+
                           |
                           v
              +------------------------+
              |  ipf_survey::rake()    |
              +------------------------+
                           |
                           v
                     Vec<f64> weights
```

### Step 1: Classify Records

For each data line, for each `WeightTable`:
- Evaluate each category's `WeightCondition` against the record.
- All matching categories are collected.
- Exactly one must match: error if 0 (`Unclassified`) or 2+
  (`MultipleMatches`).

This produces a flat `Vec<usize>` of codes: `codes[record * n_tables + table]`.

### Step 2: Build ipf_survey Structures

One `Variable` per weight table. `CodedSurvey` is constructed from the flat
codes array, with optional base weights extracted from an RFL field.

Target validation uses `validate_with_tolerance()` when `target_tolerance` is
set, allowing rounded production targets to pass consistency checks.

### Step 3: Rake and Return

```rust
let result = ipf_survey::rake(&survey, &validated, &config.raking)?;
```

## Public API

### Core Functions

```rust
/// Classify records and run raking. Returns one weight per record.
pub fn compute_weights(
    scheme: &WeightScheme,
    rfl: Option<&RflFile>,
    data: &[impl AsRef<str>],
) -> Result<Vec<f64>, WeightError>;

/// Classify records against a scheme. Returns coded survey + targets
/// ready for raking.
pub fn classify(
    scheme: &WeightScheme,
    rfl: Option<&RflFile>,
    data: &[impl AsRef<str>],
) -> Result<(CodedSurvey, ValidatedTargets), WeightError>;

/// Rake a pre-classified survey. Returns one weight per record.
pub fn rake_classified(
    survey: &CodedSurvey,
    targets: &ValidatedTargets,
    config: &RakingConfig,
) -> Result<Vec<f64>, WeightError>;

/// Rake a pre-classified survey. Returns the full RakingResult including
/// convergence report and diagnostics.
pub fn rake_classified_full(
    survey: &CodedSurvey,
    targets: &ValidatedTargets,
    config: &RakingConfig,
) -> Result<RakingResult, WeightError>;
```

### Multi-Pass (Split-Sample) Weighting

```rust
/// Result from compute_weights_multi_pass.
pub struct MultiPassResult {
    /// One weight per record: Some(w) if raked, None if unqualified.
    pub weights: Vec<Option<f64>>,
    /// Convergence report for each pass.
    pub pass_results: Vec<RakingResult>,
}

/// Compute weights for qualified/split-sample designs.
///
/// Each pass applies to a different subset of records (determined by
/// evaluating pass.qualifier) and is raked independently with its own
/// tables and TOTAL. If a record matches multiple passes, the last
/// matching pass wins.
pub fn compute_weights_multi_pass(
    passes: &[QualifiedWeightPass],
    tables: &[WeightTable],
    config: &WeightConfig,
    rfl: Option<&RflFile>,
    data: &[impl AsRef<str>],
) -> Result<MultiPassResult, WeightError>;
```

## Error Types

```rust
pub enum WeightError {
    /// Condition evaluation failure on a specific record.
    ConditionEval { table_id: u16, row_label: String, record: usize, source: String },

    /// Record matched no category in a table.
    Unclassified { table_id: u16, record: usize },

    /// Record matched multiple categories in a table.
    MultipleMatches { table_id: u16, record: usize, matched: Vec<String> },

    /// Base weight field not found in RFL or value couldn't be parsed.
    BaseWeightField { field: String, record: usize, detail: String },

    /// Target values don't sum consistently across tables.
    TargetMismatch { source: RakingError },

    /// Raking did not converge or another raking-layer failure.
    RakingFailed { source: RakingError },

    /// A weight pass references a table ID that doesn't exist.
    MissingTable { table_id: u16 },
}
```

## uncle.rs

`UncleExpr` parses and evaluates all UNCLE qualification syntax used in
production weight tables. It is an enum with recursive descent parsing:

```rust
pub enum UncleExpr {
    Term(Term),              // pos relation code_spec
    RQual(RQual),            // R() numeric field qualifier
    Literal(LiteralMatch),   // pos 'STRING'
    Not(Box<Self>),          // NOT(expr)
    And(Box<Self>, Box<Self>),
    Or(Box<Self>, Box<Self>),
}
```

Operator precedence: OR (lowest) < AND < NOT (unary) < primary.

Public API:

```rust
UncleExpr::parse(input: &str) -> Result<UncleExpr, ParseError>
UncleExpr::evaluate(&self, record: &str) -> Result<bool, String>
```

Evaluation is position-based against fixed-width records (1-indexed columns).
Out-of-range columns are treated as blank.

---

## parse_e.rs — .E File Parser

Parses UNCLE `.E` files to extract weight specifications for a given control
table ID.

### Public API

```rust
pub fn parse_e_file(path: &Path, control_table_id: u16)
    -> Result<ParsedWeightSpec, ParseEError>;

pub fn parse_e_content(content: &str, control_table_id: u16)
    -> Result<ParsedWeightSpec, ParseEError>;
```

### Parsed Output

```rust
pub struct ParsedWeightSpec {
    pub passes: Vec<QualifiedWeightPass>,
    pub tables: Vec<WeightTable>,
    pub assignments: Vec<ColumnAssignment>,
    pub moves: Vec<MoveDirective>,
    /// Column range for the current/prior weight, parsed from
    /// X IF(ALL) CWEIGHT(F!col:col). Used as base weights when
    /// a pass has retain = true.
    pub cweight_cols: Option<(usize, usize)>,
}

pub struct QualifiedWeightPass {
    pub qualifier: Option<UncleExpr>,   // X SET QUAL(expr) filter
    pub directive: WeightDirective,      // X WEIGHT columns + tables
    pub qual_str: Option<String>,        // original qualifier string for display
}

pub struct WeightDirective {
    pub table_ids: Vec<u16>,
    pub col_start: usize,               // 1-based
    pub col_end: usize,                 // 1-based, inclusive
    pub decimal_width: usize,
    pub total: Option<f64>,
    /// When true, the existing weight (from cweight_cols) is used as the
    /// base weight for raking rather than starting from 1.0.
    pub retain: bool,
}

/// Column copy from X MOVE fp:lp TO rp.
pub struct MoveDirective {
    pub from_start: usize,  // 1-based
    pub from_end: usize,    // 1-based, inclusive
    pub to_start: usize,    // 1-based
}

/// Conditional column assignment from X IF(condition) 1!col=value.
pub struct ColumnAssignment {
    pub condition: UncleExpr,
    pub cond_str: String,
    pub col: usize,
    pub value: String,
}
```

### Parsing Rules

1. **TABLE lines**: `TABLE <id>` starts a new table block. If a table ID
   appears more than once, the last definition wins (matches Uncle behavior).
2. **R lines**: `R <label>;<condition>;VALUE <target>` defines a category.
   - Semicolons separate the three fields.
   - The condition field is parsed via `UncleExpr::parse()`.
   - Commented rows (`*R ...`) are skipped.
3. **X WEIGHT**: Supports flexible table ID syntax before `TO`:
   - Single ID: `X WEIGHT 601 TO ...`
   - Range: `X WEIGHT 601 TH 610 TO ...`
   - Mixed: `X WEIGHT 601 603 TH 606 608 TH 610 TO ...`
   - Optional `RETAIN` keyword: use prior weight (from `cweight_cols`) as
     base weight for raking instead of 1.0.
   - `X WEIGHT UNWEIGHT` is ignored (clears prior weights in Uncle).
4. **X SET QUAL(expr)**: Sets a qualifier for subsequent `X WEIGHT` lines.
   `X SET QUAL()` or `X SET QUAL(ALL)` clears the qualifier.
5. **X IF(ALL) CWEIGHT(F!col:col)**: Declares the column range where the
   current/prior weight lives. Parsed into `cweight_cols`. The `F` (or
   other format) prefix is stripped; only the column range is retained.
6. **X MOVE fp:lp TO rp**: Column copy directive. Copies bytes from columns
   `fp..=lp` to `rp..(rp + lp - fp)` for every record. Applied as a pre-pass
   before raking, so that unqualified records get the prior weight copied to
   the output location. The `1!` prefix is stripped if present.
7. **X IF(condition) 1!col=value**: Conditional column assignments, used to
   set fixed values for records excluded from raking.
8. **Comment lines**: Lines starting with `*` are skipped.
9. **X lines** (other directives): `X ENTER NOADD`, `X SH`, `X SET OUT FILE`,
   `X PUNCH CHAR` are ignored.
10. **Table block ends**: At the next `TABLE` line or end of file.

### Error Type

```rust
pub struct ParseEError {
    pub line_num: usize,    // 0 for file-level errors
    pub message: String,
}
```

---

## weight.rs Binary

### Overview

CLI binary that reads a weighting control table from an UNCLE `.E` file, parses
the referenced weight tables, reads a fixed-width data file, runs raking via
`ipf.rs`, and writes an output data file with computed weights punched at the
designated column location.

### UNCLE Weight Table Format

Weight tables live inside `.E` files (e.g., `NAT.E`). The control table
(TABLE 600) specifies which tables to rake, where to write the weight, and
optional scaling:

```
TABLE 600
X ENTER NOADD
X SH 'DEL NAT.C'
X SET OUT FILE 'NAT.WT' OFF
X WEIGHT 601 TH 610 TO 1!2000:2006 4 OFF
X SET OUT FILE 'NAT.WT' CLOSE
X PUNCH CHAR
```

Individual weight tables contain category rows with UNCLE qualifications:

```
TABLE 601
R MALE;1!43-1;VALUE .48
R FEMALE//OTHER;1!43-2:3;VALUE .52
```

Split-sample designs use `X SET QUAL` to filter records per pass:

```
TABLE 600
X SET QUAL(1!87-1)
X WEIGHT 601 TH 602 TO 1!2000:2006 4 OFF TOTAL 2000
X SET QUAL(1!87-2)
X WEIGHT 601 TH 602 TO 1!2000:2006 4 OFF TOTAL 2000
```

RETAIN with CWEIGHT uses a prior weight as the base for raking:

```
TABLE 620
X IF(ALL) CWEIGHT(F!2400:2406)
X IF(1!70N1) 1!2316=1
X SET QUAL(1!70-1)
X WEIGHT 621 TH 628 TO 1!2410:2416 4 OFF TOTAL 3200 RETAIN
X SET QUAL(ALL)
```

Here `CWEIGHT(F!2400:2406)` declares that the prior weight lives at cols
2400-2406 (floating-point format). `RETAIN` on the `X WEIGHT` line tells the
raking engine to multiply by the existing weight rather than starting from 1.0.
Only records matching the qualifier (`1!70-1`) participate in raking; the
`X IF(1!70N1)` assignment sets a fixed value for excluded records.

### CLI Design

```
weight <table_id> [OPTIONS]
```

| Argument / Flag               | Short | Default        | Description                                    |
|-------------------------------|-------|----------------|------------------------------------------------|
| `<table_id>`                  |       | (required)     | Control table ID, e.g., `600`                  |
| `--e-file <path>`             | `-e`  | auto-discover  | Path to `.E` file                              |
| `--data-file <path>`          | `-d`  | auto-discover  | Path to data file (`.c`, `.fin`)               |
| `--layout-file <path>`        | `-l`  | auto-discover  | Path to `.rfl` file (optional)                 |
| `--output <path>`             | `-o`  | `<stem>.WT`    | Output data file path                          |

Auto-discovery searches the working directory for files with the expected
extensions. The RFL layout file is optional and only needed when CFMC conditions
or base weight fields are used.

### Processing Pipeline

```
  +--------------------------------------------------+
  |  1. Parse .E file                                |
  |     - Find TABLE <table_id> (e.g., TABLE 600)   |
  |     - Extract X WEIGHT directives, X SET QUAL    |
  |       qualifiers, X IF assignments, CWEIGHT      |
  |     - Detect RETAIN on X WEIGHT lines            |
  |     - Parse referenced weight tables             |
  +-------------------------+------------------------+
                            |
                            v
  +--------------------------------------------------+
  |  2. Load data                                    |
  |     - Read RFL file (optional)                   |
  |     - Read data file into Vec<String>            |
  +-------------------------+------------------------+
                            |
                            v
  +--------------------------------------------------+
  |  3. Apply X MOVE directives (pre-pass)           |
  |     - Copy columns for every record              |
  |     - e.g. MOVE 2400:2406 TO 2410 copies prior   |
  |       weight to the output location              |
  +-------------------------+------------------------+
                            |
                            v
  +--------------------------------------------------+
  |  4. Classify + Rake                              |
  |     If RETAIN + CWEIGHT: set base_weight_columns |
  |     If multi-pass (qualifiers present):           |
  |       compute_weights_multi_pass()               |
  |     Otherwise:                                    |
  |       classify() + rake_classified_full()        |
  +-------------------------+------------------------+
                            |
                            v
  +--------------------------------------------------+
  |  5. Apply column assignments (X IF directives)   |
  +-------------------------+------------------------+
                            |
                            v
  +--------------------------------------------------+
  |  6. Write output                                 |
  |     - Copy each data line                        |
  |     - Punch formatted weight at col_start:col_end|
  |     - Apply X IF assignments to qualifying lines |
  |     - Pad/extend lines as needed                 |
  +--------------------------------------------------+
```

### Weight Formatting

The weight value is written as a zero-padded fixed-width decimal string.

Given `col_start=2000`, `col_end=2006`, `decimal_width=4`:
- Field width = 7 characters
- Format: `{value:0>7.4}` -> e.g., `01.0234`, `12.3456`

### Summary / Status Reporting

After successful weighting, the binary prints a summary to stderr:

```
Weight tables: 601, 602, ..., 610
Output location: cols 2000-2006 (4 decimal places, 7 chars wide)
  Table 601:  2 categories, targets sum = 1.00
  Table 602:  8 categories, targets sum = 1.00
  ...
Records: 2847
Raking converged in 12 iterations (residual: 1.23e-11)

Weighted records: 2847 / 2847
Weight range: 0.3421 - 3.1876
Weight mean:  1.0000
Weight sum:   2847.00

Done. Wrote 2847 records to NAT.WT
```

---

## Design Principles

1. **UNCLE-native** -- Production weight tables use UNCLE syntax exclusively.
   `UncleExpr` handles all condition parsing and evaluation. The `.E` file
   parser always produces `WeightCondition::Uncle`. CFMC support is retained
   for programmatic use via the `WeightCondition` enum.

2. **Faithful to UNCLE semantics** -- The `X WEIGHT` directive, table ID
   ranges (`TH`), `X SET QUAL` qualifiers, `X IF` assignments, `CWEIGHT` /
   `RETAIN` base weights, output location, decimal formatting, and `TOTAL`
   scaling all mirror UNCLE behavior.

3. **Fail loud** -- Errors name the table ID, row label, and record index.
   Parse errors in the `.E` file report line numbers.

4. **Separation of concerns** -- `uncle.rs` parses/evaluates UNCLE
   expressions. `parse_e.rs` handles `.E` file structure. `ipf.rs` handles
   classification and raking. `weight.rs` orchestrates I/O.

5. **Consistent CLI** -- Follows the same `clap` / auto-discover pattern as
   `freq` and `banners`.
