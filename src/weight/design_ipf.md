# ipf.rs Design Spec

## Overview

`ipf.rs` is a glue layer within `dp_library` that connects survey data
(fixed-width records), question metadata (RFL layouts + CFMC logic), and the
`ipf_survey` raking engine. It accepts weight table definitions with CFMC
conditions, builds the structures `ipf_survey` expects, and returns a `Vec<f64>`
of per-record weights.

## Location

All new code lives within `dp_library` at `/home/alexm/rust/vd/dp_library/`.

### Directory Structure

```
dp_library/src/
  lib.rs            - add: pub mod weight;
  cfmc.rs           - existing
  rfl.rs            - existing
  crosstabs.rs      - existing
  weight/
    mod.rs          - public re-exports
    ipf.rs          - glue layer: CFMC condition evaluation, CodedSurvey
                      construction, raking orchestration, Vec<f64> output
    uncle.rs        - (future) UNCLE-native syntax parser for R(), NOT(),
                      1!col-punch, raw column access, implicit AND
```

### Cargo.toml Addition

```toml
[dependencies]
ipf_survey = { path = "../ipf_survey" }
```

A new binary target for the weight tool:

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
struct WeightScheme {
    tables: Vec<WeightTable>,
    config: WeightConfig,
}

struct WeightTable {
    id: u16,                            // e.g., 601
    label: Option<String>,              // optional human-readable name
    categories: Vec<WeightCategory>,
}

struct WeightCategory {
    label: String,          // e.g., "MALE", "NEW ENGLAND"
    condition: CfmcLogic,   // parsed CFMC expression
    target: f64,            // the VALUE (target proportion)
}
```

### 2. Conditions — CFMC Only (First Pass)

All conditions are CFMC expressions evaluated via `CfmcLogic`. Weight table
rows use question labels from the RFL:

```
R MALE;GENDER(1);VALUE .48
R FEMALE//OTHER;GENDER(2,3);VALUE .52
```

The syntax `GENDER(1)` is parsed by `CfmcLogic::parse()` and evaluated via
`CfmcLogic::evaluate()` against the RFL question map and a data line.

### What the First Pass Covers

| Syntax                        | Example                              | Supported? |
|-------------------------------|--------------------------------------|------------|
| CFMC expression               | `GENDER(1)`                          | Yes        |
| CFMC with code range          | `GENDER(2,3)` / `AGEGROUP(1-3)`     | Yes        |
| CFMC AND/OR/NOT               | `QD3A(6) AND VERSION(1)`            | Yes        |
| CFMC NOT                      | `NOT QE(1)`                          | Yes        |
| Single-field UNCLE ref        | `1!43-1`                             | No*        |
| Code range (UNCLE)            | `1!43-1:3`                           | No*        |
| Code list (UNCLE)             | `1!194-8,9`                          | No*        |
| Implicit AND (UNCLE)          | `1!48-6 1!51-1`                      | No*        |
| NOT prefix (UNCLE)            | `NOT(1!44-1)`                        | No*        |
| Inline NOT (UNCLE)            | `1!49N1`                             | No*        |
| R() range qualifier           | `R(1!26:27,2,6,15)`                  | No*        |
| Raw column access             | `1!1949 'R'`                         | No*        |
| Character literal match       | `1!1949 'U'`                         | No*        |

*All UNCLE syntax deferred to `weight/uncle.rs`

### 3. Survey Data

- An `RflFile` for question metadata and field extraction.
- A `Vec<String>` (or iterator) of fixed-width data lines, one per respondent.

### 4. Configuration

```rust
struct WeightConfig {
    /// Passed through to ipf_survey::rake()
    raking: RakingConfig,

    /// Optional base weight field — an RFL question label whose numeric value
    /// is the respondent's design weight. If None, all base weights are 1.0.
    base_weight_field: Option<String>,
}
```

## Processing Pipeline

```
                   ┌──────────────┐
                   │ WeightScheme │  (tables + config)
                   └──────┬───────┘
                          │
                          ▼
              ┌───────────────────────┐
              │  Classify Records     │  For each respondent, evaluate each
              │                       │  table's CFMC conditions → category index
              └───────────┬───────────┘
                          │
                          ▼
              ┌───────────────────────┐
              │  Build CodedSurvey    │  N records × K tables matrix
              │  + PopulationTargets  │  One Variable per table
              └───────────┬───────────┘
                          │
                          ▼
              ┌───────────────────────┐
              │  ipf_survey::rake()   │
              └───────────┬───────────┘
                          │
                          ▼
                    Vec<f64> weights
```

### Step 1: Classify Records

For each data line, for each `WeightTable`:
- Evaluate each category's `CfmcLogic` condition against the RFL questions and
  the data line.
- The first matching category determines the record's code for that variable.
- If no category matches → error (report table ID and record index).

This produces a flat `Vec<usize>` of codes: `codes[record * n_tables + table]`.

**Validation**: Categories within a table must be mutually exclusive and
exhaustive. The library should warn if a record matches multiple categories and
error if it matches none.

### Step 2: Build ipf_survey Structures

```rust
// One Variable per weight table
let variables: Vec<Variable> = tables.iter().map(|t| Variable {
    name: format!("TABLE_{}", t.id),
    levels: t.categories.len(),
    labels: Some(t.categories.iter().map(|c| c.label.clone()).collect()),
}).collect();

// Targets: one entry per table
let mut targets = PopulationTargets::new();
for table in &tables {
    targets.add(
        format!("TABLE_{}", table.id),
        table.categories.iter().map(|c| c.target).collect(),
    );
}

let survey = CodedSurvey::from_flat_codes_weighted(
    variables, codes, n_records, base_weights,
);
```

### Step 3: Rake and Return

```rust
let validated = targets.validate(&survey)?;
let result = ipf_survey::rake(&survey, &validated, &config.raking)?;
// result.weights is the Vec<f64>
```

## Public API

```rust
/// Classify records and run raking. Returns one weight per record.
pub fn compute_weights(
    scheme: &WeightScheme,
    rfl: &RflFile,
    data: &[String],
) -> Result<Vec<f64>, WeightError>;
```

Optionally, a two-phase API for reuse across multiple data files:

```rust
/// Classify records against a scheme. Returns coded survey + targets
/// ready for raking.
pub fn classify(
    scheme: &WeightScheme,
    rfl: &RflFile,
    data: &[String],
) -> Result<(CodedSurvey, ValidatedTargets), WeightError>;

/// Rake a pre-classified survey.
pub fn rake_classified(
    survey: &CodedSurvey,
    targets: &ValidatedTargets,
    config: &RakingConfig,
) -> Result<Vec<f64>, WeightError>;
```

## Error Types

```rust
enum WeightError {
    /// CFMC parse failure
    ConditionParse { table_id: u16, row_label: String, source: String },

    /// CFMC evaluation failure on a specific record
    ConditionEval { table_id: u16, row_label: String, record: usize, source: String },

    /// Record matched no category in a table
    Unclassified { table_id: u16, record: usize },

    /// Record matched multiple categories in a table
    MultipleMatches { table_id: u16, record: usize, matched: Vec<String> },

    /// Target values don't sum consistently across tables
    TargetMismatch { source: ipf_survey::RakingError },

    /// Raking did not converge
    RakingFailed { source: ipf_survey::RakingError },
}
```

## Completed: uncle.rs

`uncle.rs` is implemented. `UncleExpr` parses and evaluates all UNCLE
qualification syntax used in production weight tables:

- `1!43-1`, `1!43-1:3`, `1!194-8,9` — position/code tests
- `1!48-6 1!51-1` — implicit AND
- `1!49N1`, `NOT(1!44-1)` — negation
- `R(1!26:27,2,6,15,41,53)` — multi-column numeric range qualifiers
- `1!1949 'R'` — literal character match

Public API:

```rust
UncleExpr::parse(input: &str) -> Result<UncleExpr, ParseError>
UncleExpr::evaluate(&self, record: &str) -> Result<bool, String>
```

## Completed: ipf.rs

The raking glue layer is implemented. `WeightCategory.condition` currently uses
`CfmcLogic`, but the next step requires switching to UNCLE conditions since
production weight tables use UNCLE syntax exclusively.

---

# Phase 3: weight.rs Binary

## Overview

`weight.rs` is a CLI binary that reads a weighting control table (TABLE 600)
from an UNCLE `.E` file, parses the referenced weight tables (601–610), reads
a fixed-width data file, runs raking via `ipf.rs`, and writes an output data
file with computed weights punched at the designated column location.

## UNCLE Weight Table Format

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

```
TABLE 602
R NEW ENGLAND;R(1!26:27,9,23,25,33,44,50);VALUE .04750
R MID ATLANTIC;R(1!26:27,10,11,24,34,36,42,54);VALUE .16000
...
```

### Row Format

```
R <label>;<uncle_condition>;VALUE <target>
```

- **label**: Human-readable category name (`//` separates alternates)
- **uncle_condition**: UNCLE qualification parsed by `UncleExpr::parse()`
- **target**: Population target proportion (f64)

Comment rows (`*`) and commented-out rows (`*R ...`) are ignored.

### X WEIGHT Directive

```
X WEIGHT <start> TH <end> TO 1!<col_start>:<col_end> <decimal_width> OFF [TOTAL <n>]
```

| Field           | Example     | Meaning                                        |
|-----------------|-------------|------------------------------------------------|
| `start`         | `601`       | First weight table ID                          |
| `end`           | `610`       | Last weight table ID                           |
| `col_start`     | `2000`      | Output column start (1-based)                  |
| `col_end`       | `2006`      | Output column end (1-based, inclusive)          |
| `decimal_width` | `4`         | Digits after the decimal point                 |
| `TOTAL n`       | `TOTAL 800` | Optional: scale weights so they sum to `n`     |

The output field width = `col_end - col_start + 1` (e.g., 2000:2006 = 7 chars).
The weight is formatted as a fixed-width decimal: width 7, 4 decimal places →
`_1.2345` (leading space, or `12.3456` for larger values).

When `TOTAL` is absent, weights are proportional (sum to 1.0 by default from
raking). When `TOTAL n` is present, weights are scaled so their sum = `n`.
In practice, `TOTAL 800` with 800 respondents produces weights that average 1.0.

## CLI Design

```
weight <table_id> [OPTIONS]
```

### Arguments

| Argument       | Required | Description                                      |
|----------------|----------|--------------------------------------------------|
| `<table_id>`   | yes      | Control table ID, e.g., `600` (runs `ex600`)     |

### Options

| Flag                  | Short | Default        | Description                                    |
|-----------------------|-------|----------------|------------------------------------------------|
| `--exec-file <path>`  | `-e`  | auto-discover  | Path to `.E` file                              |
| `--data-file <path>`  | `-d`  | auto-discover  | Path to input data file (`.fin`, `.rft`, `.c`) |
| `--layout-file <path>`| `-l`  | auto-discover  | Path to `.rfl` file                            |
| `--output <path>`     | `-o`  | `<stem>.WT`    | Output data file path                          |

Auto-discovery follows the existing pattern from `freq`/`banners`: search the
working directory for files with the expected extension.

### Example Usage

```bash
# Minimal — auto-discovers NAT.E, p0157.rfl, and data file
weight 600

# Explicit paths
weight 600 -e NAT.E -d NAT.fin -l p0157.rfl -o NAT.WT
```

## Processing Pipeline

```
  ┌──────────────────────────────────────────────────┐
  │  1. Parse .E file                                │
  │     - Find TABLE <table_id> (e.g., TABLE 600)    │
  │     - Extract X WEIGHT directive → table range,   │
  │       output location, decimal width, TOTAL       │
  │     - Parse TABLE 601..610 → Vec<WeightTable>     │
  └──────────────────────┬───────────────────────────┘
                         │
                         ▼
  ┌──────────────────────────────────────────────────┐
  │  2. Load data                                    │
  │     - Read RFL file                              │
  │     - Read data file into Vec<String>            │
  └──────────────────────┬───────────────────────────┘
                         │
                         ▼
  ┌──────────────────────────────────────────────────┐
  │  3. Build WeightScheme                           │
  │     - WeightCategory conditions use UncleExpr    │
  │     - Targets from VALUE directives              │
  └──────────────────────┬───────────────────────────┘
                         │
                         ▼
  ┌──────────────────────────────────────────────────┐
  │  4. Classify + Rake                              │
  │     - classify() → CodedSurvey + targets         │
  │     - rake_classified() → Vec<f64> weights       │
  └──────────────────────┬───────────────────────────┘
                         │
                         ▼
  ┌──────────────────────────────────────────────────┐
  │  5. Scale (optional)                             │
  │     - If TOTAL n: scale weights so sum = n       │
  └──────────────────────┬───────────────────────────┘
                         │
                         ▼
  ┌──────────────────────────────────────────────────┐
  │  6. Write output                                 │
  │     - Copy each data line                        │
  │     - Overwrite cols col_start..col_end with     │
  │       formatted weight value                     │
  │     - Pad/extend lines if weight location >      │
  │       current line length                        │
  └──────────────────────────────────────────────────┘
```

## Required Changes to ipf.rs

### WeightCategory Condition Type

`WeightCategory.condition` must support UNCLE syntax. Replace `CfmcLogic` with
a dispatch enum:

```rust
pub enum WeightCondition {
    Cfmc(CfmcLogic),
    Uncle(UncleExpr),
}

impl WeightCondition {
    /// Evaluate against a data record.
    pub fn evaluate(
        &self,
        questions: &AHashMap<String, RflQuestion>,
        record: &str,
    ) -> Result<bool, String> {
        match self {
            Self::Cfmc(logic) => logic.evaluate(questions, record),
            Self::Uncle(expr) => expr.evaluate(record),
        }
    }
}

pub struct WeightCategory {
    pub label: String,
    pub condition: WeightCondition,
    pub target: f64,
}
```

The `classify()` function calls `category.condition.evaluate()` unchanged —
the dispatch is internal to `WeightCondition`.

## .E File Parser

A new module `weighting/parse_e.rs` (or a section within `weight.rs`) handles
parsing the `.E` file to extract weight table definitions.

### Parsing Rules

1. **TABLE lines**: `TABLE <id>` starts a new table block.
2. **R lines**: `R <label>;<condition>;VALUE <target>` defines a category.
   - Semicolons separate the three fields.
   - The condition field is parsed via `UncleExpr::parse()`.
   - Commented rows (`*R ...`) are skipped.
3. **X WEIGHT line**: Parsed for table range, output location, decimal width,
   and optional TOTAL.
4. **Comment lines**: Lines starting with `*` (that aren't `*R`) are skipped.
5. **X lines** (other than X WEIGHT): Ignored — `X ENTER NOADD`,
   `X SH`, `X SET OUT FILE`, `X PUNCH CHAR` are Uncle exec directives not
   relevant to weight computation.
6. **Table block ends**: At the next `TABLE` line, end of file, or `*` line
   between tables.

### Parsed Output

```rust
pub struct WeightDirective {
    pub table_start: u16,       // 601
    pub table_end: u16,         // 610
    pub col_start: usize,       // 2000 (1-based)
    pub col_end: usize,         // 2006 (1-based)
    pub decimal_width: usize,   // 4
    pub total: Option<f64>,     // None or Some(800.0)
}
```

## Weight Formatting

The weight value is written as a right-justified fixed-width decimal string.

Given `col_start=2000`, `col_end=2006`, `decimal_width=4`:
- Field width = 7 characters
- Format: `%07.4f` equivalent → e.g., ` 01.0234`, `12.3456`
- If the formatted value exceeds the field width, it fills or overflows (error).

## Output File

- Each input data line is copied to the output.
- Lines shorter than `col_end` are padded with spaces to reach the weight
  location.
- The weight value is overwritten at columns `col_start..=col_end`
  (1-based, so byte indices `col_start-1..col_end`).
- The output file is written as a new file (default `<stem>.WT`).

## Summary / Status Reporting

After successful weighting, print a summary to stderr:

```
Weight tables: 601-610
Records: 2847
Output location: cols 2000-2006 (4 decimal places)
Output file: NAT.WT

Table 601 (GENDER):        2 categories, targets sum = 1.00
Table 602 (REGION):        8 categories, targets sum = 1.00
...
Raking converged in 12 iterations.
Weight range: 0.3421 - 3.1876
Weight mean:  1.0000
```

## Design Principles

1. **UNCLE-native** — Production weight tables use UNCLE syntax exclusively.
   `UncleExpr` handles all condition parsing and evaluation. CFMC support is
   retained for flexibility via the `WeightCondition` enum.

2. **Faithful to UNCLE semantics** — The `X WEIGHT` directive, table range,
   output location, decimal formatting, and TOTAL scaling all mirror UNCLE
   behavior.

3. **Fail loud** — Errors name the table ID, row label, and record index.
   Parse errors in the `.E` file report line numbers.

4. **Separation of concerns** — `ipf.rs` handles classification and raking.
   `.E` file parsing is a separate concern. `weight.rs` orchestrates I/O.

5. **Consistent CLI** — Follows the same `clap` / auto-discover pattern as
   `freq` and `banners`.
