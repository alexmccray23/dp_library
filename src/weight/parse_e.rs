use std::fmt;
use std::fs;
use std::path::Path;

use crate::weight::{UncleExpr, WeightCategory, WeightCondition, WeightTable};

// ---------------------------------------------------------------------------
// Public data structures
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct WeightDirective {
    pub table_ids: Vec<u16>,
    pub col_start: usize,
    pub col_end: usize,
    pub decimal_width: usize,
    pub total: Option<f64>,
}

impl WeightDirective {
    #[must_use]
    pub const fn field_width(&self) -> usize {
        self.col_end - self.col_start + 1
    }
}

/// A single weighting pass with an optional record qualifier.
///
/// When `qualifier` is `Some`, only records matching the expression are
/// included in this pass's raking. Multiple passes with disjoint qualifiers
/// allow split-sample weighting (e.g. two half-samples each weighted to their
/// own TOTAL independently).
///
/// **Callers are responsible for ensuring qualifiers are mutually exclusive when
/// records should receive exactly one weight.** The parser does not enforce this.
#[derive(Debug)]
pub struct QualifiedWeightPass {
    pub qualifier: Option<UncleExpr>,
    pub directive: WeightDirective,
}

/// A conditional column assignment from an `X IF(condition) col=value` directive.
///
/// Uncle uses these to set fixed values for records that don't participate in
/// raking (e.g. assigning weight 1 to non-registered voters).
#[derive(Debug)]
pub struct ColumnAssignment {
    pub condition: UncleExpr,
    pub col: usize,
    pub value: String,
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct ParseEError {
    pub line_num: usize,
    pub message: String,
}

impl fmt::Display for ParseEError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line_num == 0 {
            write!(f, "{}", self.message)
        } else {
            write!(f, "line {}: {}", self.line_num, self.message)
        }
    }
}

impl std::error::Error for ParseEError {}

// ---------------------------------------------------------------------------
// Parsed result
// ---------------------------------------------------------------------------

pub struct ParsedWeightSpec {
    pub passes: Vec<QualifiedWeightPass>,
    pub tables: Vec<WeightTable>,
    pub assignments: Vec<ColumnAssignment>,
}

impl ParsedWeightSpec {
    /// Convenience accessor for single-pass specs (the common case).
    ///
    /// # Panics
    ///
    /// Panics if the spec has zero or multiple passes.
    #[must_use]
    pub fn directive(&self) -> &WeightDirective {
        assert_eq!(self.passes.len(), 1, "expected single pass, got {}", self.passes.len());
        &self.passes[0].directive
    }
}

impl fmt::Debug for ParsedWeightSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ParsedWeightSpec")
            .field("passes", &self.passes.len())
            .field("tables_count", &self.tables.len())
            .field("assignments", &self.assignments.len())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Read an `.E` file and extract the weight specification for the given
/// control table ID (e.g., 600). Returns the `X WEIGHT` directive and all
/// referenced weight tables.
pub fn parse_e_file(path: &Path, control_table_id: u16) -> Result<ParsedWeightSpec, ParseEError> {
    let content = fs::read_to_string(path).map_err(|e| ParseEError {
        line_num: 0,
        message: format!("could not read {}: {e}", path.display()),
    })?;
    parse_e_content(&content, control_table_id)
}

/// Parse `.E` file content and extract the weight specification.
pub fn parse_e_content(content: &str, control_table_id: u16) -> Result<ParsedWeightSpec, ParseEError> {
    let lines: Vec<&str> = content.lines().collect();

    // First pass: index all TABLE blocks by ID → (start_line, end_line).
    let table_blocks = index_table_blocks(&lines);

    // Find the control table and extract qualified weight passes.
    let control_range = table_blocks
        .iter()
        .find(|b| b.id == control_table_id)
        .ok_or_else(|| ParseEError {
            line_num: 0,
            message: format!("TABLE {control_table_id} not found in .E file"),
        })?;

    let (passes, assignments) = extract_weight_passes(&lines, control_range)?;

    // Collect the union of all table IDs across passes.
    let mut seen_ids = Vec::new();
    for pass in &passes {
        for &id in &pass.directive.table_ids {
            if !seen_ids.contains(&id) {
                seen_ids.push(id);
            }
        }
    }

    // Parse each weight table.
    let mut tables = Vec::new();
    for &table_id in &seen_ids {
        let block = table_blocks
            .iter()
            .find(|b| b.id == table_id)
            .ok_or_else(|| ParseEError {
                line_num: 0,
                message: format!("TABLE {table_id} referenced by X WEIGHT but not found in .E file"),
            })?;
        let table = parse_weight_table(&lines, block)?;
        tables.push(table);
    }

    Ok(ParsedWeightSpec { passes, tables, assignments })
}

// ---------------------------------------------------------------------------
// Internal: table block indexing
// ---------------------------------------------------------------------------

struct TableBlock {
    id: u16,
    start: usize, // line index of "TABLE <id>"
    end: usize,   // exclusive end (next TABLE line or EOF)
}

fn index_table_blocks(lines: &[&str]) -> Vec<TableBlock> {
    let mut blocks = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if let Some(id) = parse_table_line(lines[i]) {
            let start = i;
            i += 1;
            while i < lines.len() && parse_table_line(lines[i]).is_none() {
                i += 1;
            }
            // If a table ID appears more than once, keep the last definition
            // (matches Uncle's behavior where later definitions override earlier ones).
            if let Some(pos) = blocks.iter().position(|b: &TableBlock| b.id == id) {
                blocks[pos] = TableBlock { id, start, end: i };
            } else {
                blocks.push(TableBlock { id, start, end: i });
            }
        } else {
            i += 1;
        }
    }
    blocks
}

fn parse_table_line(line: &str) -> Option<u16> {
    let trimmed = line.trim();
    let rest = trimmed.strip_prefix("TABLE")?;
    if rest.is_empty() || !rest.starts_with(|c: char| c.is_ascii_whitespace()) {
        return None;
    }
    rest.trim().parse::<u16>().ok()
}

// ---------------------------------------------------------------------------
// Internal: X WEIGHT directive
// ---------------------------------------------------------------------------

/// Scan the control table for `X SET QUAL(...)`, `X WEIGHT`, and `X IF`
/// directives. Produces one `QualifiedWeightPass` per `X WEIGHT` line (paired
/// with the most recently set qualifier) and a list of `ColumnAssignment`s
/// from `X IF` directives.
fn extract_weight_passes(
    lines: &[&str],
    block: &TableBlock,
) -> Result<(Vec<QualifiedWeightPass>, Vec<ColumnAssignment>), ParseEError> {
    let mut passes = Vec::new();
    let mut assignments = Vec::new();
    let mut current_qual: Option<UncleExpr> = None;

    for (i, line) in lines.iter().enumerate().take(block.end).skip(block.start) {
        let trimmed = line.trim();

        // X SET QUAL(expr) — set or clear the current qualifier.
        if let Some(rest) = trimmed.strip_prefix("X SET QUAL") {
            let rest = rest.trim();
            if let Some(inner) = rest.strip_prefix('(').and_then(|r| r.strip_suffix(')')) {
                let inner = inner.trim();
                if inner.is_empty() || inner.eq_ignore_ascii_case("ALL") {
                    current_qual = None;
                } else {
                    current_qual = Some(UncleExpr::parse(inner).map_err(|e| ParseEError {
                        line_num: i + 1,
                        message: format!("X SET QUAL parse error: {e}"),
                    })?);
                }
            } else if rest.is_empty() {
                // Bare "X SET QUAL" with no parens clears qualifier.
                current_qual = None;
            }
            continue;
        }

        // X IF(condition) col=value — conditional column assignment.
        if let Some(rest) = trimmed.strip_prefix("X IF") {
            if let Some(assignment) = parse_x_if(rest.trim(), i + 1)? {
                assignments.push(assignment);
            }
            continue;
        }

        // X WEIGHT directive.
        if let Some(rest) = trimmed.strip_prefix("X WEIGHT") {
            if rest.is_empty() || !rest.starts_with(|c: char| c.is_ascii_whitespace()) {
                continue;
            }
            let rest = rest.trim();
            if rest.eq_ignore_ascii_case("UNWEIGHT") {
                continue;
            }
            let directive = parse_weight_line(rest, i + 1)?;
            passes.push(QualifiedWeightPass {
                qualifier: current_qual.clone(),
                directive,
            });
        }
    }

    if passes.is_empty() {
        return Err(ParseEError {
            line_num: block.start + 1,
            message: format!("no X WEIGHT directive found in TABLE {}", block.id),
        });
    }
    Ok((passes, assignments))
}

/// Parse `X IF(condition) 1!col=value`. Returns `None` for forms we don't
/// handle (e.g. missing parens or unexpected syntax).
fn parse_x_if(s: &str, line_num: usize) -> Result<Option<ColumnAssignment>, ParseEError> {
    // Expect: (condition) col_spec=value
    let s = s.strip_prefix('(').unwrap_or(s);
    let Some((cond_str, rest)) = s.rsplit_once(')') else {
        return Ok(None);
    };
    let cond_str = cond_str.trim();
    if cond_str.is_empty() {
        return Ok(None);
    }

    let condition = UncleExpr::parse(cond_str).map_err(|e| ParseEError {
        line_num,
        message: format!("X IF condition parse error: {e}"),
    })?;

    // rest: " 1!2316=1" or " 2316=1"
    let rest = rest.trim();
    let rest = rest.strip_prefix("1!").unwrap_or(rest);
    let Some((col_str, value)) = rest.split_once('=') else {
        return Ok(None);
    };
    let col: usize = col_str.trim().parse().map_err(|_| ParseEError {
        line_num,
        message: format!("X IF: invalid column number '{col_str}'"),
    })?;

    Ok(Some(ColumnAssignment {
        condition,
        col,
        value: value.trim().to_string(),
    }))
}

/// Parse the tokens after `X WEIGHT`. Supported forms:
///
/// - `601 TH 610 TO 1!2000:2006 4 OFF`
/// - `601 TO 1!5060:5066 4`
/// - `601 603 TH 606 608 TH 610 TO 1!5060:5066 4`
///
/// Everything before `TO` is table IDs and ranges. After `TO`: column
/// location, decimal width, optional `OFF`, optional `TOTAL <n>`.
fn parse_weight_line(s: &str, line_num: usize) -> Result<WeightDirective, ParseEError> {
    let err = |msg: &str| ParseEError {
        line_num,
        message: format!("X WEIGHT parse error: {msg}"),
    };

    let tokens: Vec<&str> = s.split_whitespace().collect();

    // Find the TO token — everything before it describes table IDs/ranges.
    let to_pos = tokens
        .iter()
        .position(|t| t.eq_ignore_ascii_case("TO"))
        .ok_or_else(|| err("missing 'TO' keyword"))?;

    let table_ids = parse_table_id_list(&tokens[..to_pos], line_num)?;
    if table_ids.is_empty() {
        return Err(err("no table IDs before 'TO'"));
    }

    // After TO: 1!<col_range> <decimal_width> [OFF] [TOTAL <n>]
    let after_to = &tokens[to_pos + 1..];
    if after_to.len() < 2 {
        return Err(err("expected column range and decimal width after 'TO'"));
    }

    let loc_str = after_to[0].strip_prefix("1!").unwrap_or(after_to[0]);
    let (col_start, col_end) = parse_col_range(loc_str)
        .ok_or_else(|| err("invalid column range (expected e.g. 2000:2006)"))?;

    let decimal_width: usize = after_to[1]
        .parse()
        .map_err(|_| err("invalid decimal width"))?;

    // Scan remaining tokens for optional TOTAL.
    let mut total = None;
    let mut j = 2;
    while j < after_to.len() {
        if after_to[j].eq_ignore_ascii_case("TOTAL") && j + 1 < after_to.len() {
            total = Some(
                after_to[j + 1]
                    .parse::<f64>()
                    .map_err(|_| err("invalid TOTAL value"))?,
            );
            break;
        }
        j += 1;
    }

    Ok(WeightDirective {
        table_ids,
        col_start,
        col_end,
        decimal_width,
        total,
    })
}

/// Parse table IDs from tokens before `TO`.
///
/// Handles individual IDs and `<start> TH <end>` ranges:
/// - `601` → `[601]`
/// - `601 TH 610` → `[601, 602, ..., 610]`
/// - `601 603 TH 606 608 TH 610` → `[601, 603, 604, 605, 606, 608, 609, 610]`
fn parse_table_id_list(tokens: &[&str], line_num: usize) -> Result<Vec<u16>, ParseEError> {
    let err = |msg: String| ParseEError {
        line_num,
        message: format!("X WEIGHT parse error: {msg}"),
    };

    let mut ids = Vec::new();
    let mut i = 0;

    while i < tokens.len() {
        let id: u16 = tokens[i]
            .parse()
            .map_err(|_| err(format!("invalid table ID '{}'", tokens[i])))?;

        // Check if next token is TH → range.
        if i + 2 < tokens.len() && tokens[i + 1].eq_ignore_ascii_case("TH") {
            let end: u16 = tokens[i + 2]
                .parse()
                .map_err(|_| err(format!("invalid end table ID '{}'", tokens[i + 2])))?;
            ids.extend(id..=end);
            i += 3;
        } else {
            ids.push(id);
            i += 1;
        }
    }

    Ok(ids)
}

fn parse_col_range(s: &str) -> Option<(usize, usize)> {
    let (a, b) = s.split_once(':')?;
    Some((a.parse().ok()?, b.parse().ok()?))
}

// ---------------------------------------------------------------------------
// Internal: weight table rows
// ---------------------------------------------------------------------------

fn parse_weight_table(lines: &[&str], block: &TableBlock) -> Result<WeightTable, ParseEError> {
    let mut categories = Vec::new();

    for (i, line) in lines.iter().enumerate().take(block.end).skip(block.start + 1) {
        let trimmed = line.trim();

        // Skip comments and blank lines.
        if trimmed.is_empty() || trimmed.starts_with('*') {
            continue;
        }

        // Skip X directives.
        if trimmed.starts_with("X ") {
            continue;
        }

        // Parse R rows.
        if let Some(rest) = trimmed.strip_prefix("R ") {
            let cat = parse_r_row(rest.trim(), block.id, i + 1)?;
            categories.push(cat);
        }
    }

    if categories.is_empty() {
        return Err(ParseEError {
            line_num: block.start + 1,
            message: format!("TABLE {} has no R rows", block.id),
        });
    }

    Ok(WeightTable {
        id: block.id,
        label: None,
        categories,
    })
}

/// Parse: `MALE;1!43-1;VALUE .48`
fn parse_r_row(s: &str, table_id: u16, line_num: usize) -> Result<WeightCategory, ParseEError> {
    let err = |msg: String| ParseEError {
        line_num,
        message: format!("TABLE {table_id} R row: {msg}"),
    };

    let parts: Vec<&str> = s.splitn(3, ';').collect();
    if parts.len() != 3 {
        return Err(err(format!("expected 3 semicolon-separated fields, got {}", parts.len())));
    }

    let label = parts[0].trim().to_string();

    let condition_str = parts[1].trim();
    let condition = UncleExpr::parse(condition_str).map_err(|e| {
        err(format!("condition parse error: {e}"))
    })?;

    let value_str = parts[2].trim();
    let target = value_str
        .strip_prefix("VALUE")
        .or_else(|| value_str.strip_prefix("value"))
        .map(str::trim)
        .ok_or_else(|| err("expected 'VALUE <target>'".to_string()))?
        .parse::<f64>()
        .map_err(|_| err(format!("invalid target value in '{value_str}'")))?;

    Ok(WeightCategory {
        label,
        condition: WeightCondition::Uncle(condition),
        target,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_E: &str = "\
* Test weight spec
*
TABLE 600
X ENTER NOADD
X SH 'DEL ABA.C'
X SET OUT FILE 'ABA.WT' OFF
X WEIGHT 601 TH 602 TO 1!2000:2006 4 OFF
X SET OUT FILE 'ABA.WT' CLOSE
X PUNCH CHAR
*
TABLE 601
R MALE;1!43-1;VALUE .48
R FEMALE//OTHER;1!43-2:3;VALUE .52
*
TABLE 602
R 18 TO 44;1!41-1:3;VALUE .47
R 65 AND OVER;1!41-6;VALUE .20
R BALANCE;1!41-4,5,7;VALUE .33
*
TABLE 700
R SOMETHING;1!50-1;VALUE 1.0
";

    #[test]
    fn parse_control_table_and_weight_tables() {
        let spec = parse_e_content(SAMPLE_E, 600).unwrap();
        assert_eq!(spec.passes.len(), 1);
        assert!(spec.passes[0].qualifier.is_none());
        let d = spec.directive();
        assert_eq!(d.table_ids, vec![601, 602]);
        assert_eq!(d.col_start, 2000);
        assert_eq!(d.col_end, 2006);
        assert_eq!(d.decimal_width, 4);
        assert_eq!(d.field_width(), 7);
        assert!(d.total.is_none());

        assert_eq!(spec.tables.len(), 2);

        // TABLE 601
        assert_eq!(spec.tables[0].id, 601);
        assert_eq!(spec.tables[0].categories.len(), 2);
        assert_eq!(spec.tables[0].categories[0].label, "MALE");
        assert!((spec.tables[0].categories[0].target - 0.48).abs() < 1e-9);
        assert_eq!(spec.tables[0].categories[1].label, "FEMALE//OTHER");
        assert!((spec.tables[0].categories[1].target - 0.52).abs() < 1e-9);

        // TABLE 602
        assert_eq!(spec.tables[1].id, 602);
        assert_eq!(spec.tables[1].categories.len(), 3);
        assert!((spec.tables[1].categories[2].target - 0.33).abs() < 1e-9);
    }

    #[test]
    fn parse_total_directive() {
        let content = "\
TABLE 600
X WEIGHT 601 TH 601 TO 1!100:106 4 OFF TOTAL 800
*
TABLE 601
R A;1!10-1;VALUE .5
R B;1!10-2;VALUE .5
";
        let spec = parse_e_content(content, 600).unwrap();
        assert_eq!(spec.directive().total, Some(800.0));
    }

    #[test]
    fn missing_control_table_returns_error() {
        let err = parse_e_content(SAMPLE_E, 999).unwrap_err();
        assert!(err.message.contains("999"), "{err}");
    }

    #[test]
    fn missing_weight_table_returns_error() {
        let content = "\
TABLE 600
X WEIGHT 601 TH 602 TO 1!100:106 4 OFF
*
TABLE 601
R A;1!10-1;VALUE 1.0
";
        let err = parse_e_content(content, 600).unwrap_err();
        assert!(err.message.contains("602"), "{err}");
    }

    #[test]
    fn commented_rows_are_skipped() {
        let content = "\
TABLE 600
X WEIGHT 601 TH 601 TO 1!100:106 4 OFF
*
TABLE 601
R ACTIVE;1!10-1;VALUE .6
*R COMMENTED OUT;1!10-2;VALUE .1
R REST;1!10-2:3;VALUE .4
";
        let spec = parse_e_content(content, 600).unwrap();
        assert_eq!(spec.tables[0].categories.len(), 2);
    }

    #[test]
    fn parse_single_table_no_th() {
        let content = "\
TABLE 600
X WEIGHT 601 TO 1!5060:5066 4
*
TABLE 601
R A;1!10-1;VALUE .5
R B;1!10-2;VALUE .5
";
        let spec = parse_e_content(content, 600).unwrap();
        assert_eq!(spec.directive().table_ids, vec![601]);
        assert_eq!(spec.tables.len(), 1);
    }

    #[test]
    fn parse_multi_range_tables() {
        let content = "\
TABLE 600
X WEIGHT 601 603 TH 606 608 TH 610 TO 1!5060:5066 4
TABLE 601
R A;1!10-1;VALUE .5
R B;1!10-2;VALUE .5
*
TABLE 603
R A;1!11-1;VALUE .5
R B;1!11-2;VALUE .5
*
TABLE 604
R A;1!12-1;VALUE .5
R B;1!12-2;VALUE .5
*
TABLE 605
R A;1!13-1;VALUE .5
R B;1!13-2;VALUE .5
*
TABLE 606
R A;1!14-1;VALUE .5
R B;1!14-2;VALUE .5
*
TABLE 608
R A;1!15-1;VALUE .5
R B;1!15-2;VALUE .5
*
TABLE 609
R A;1!16-1;VALUE .5
R B;1!16-2;VALUE .5
*
TABLE 610
R A;1!17-1;VALUE .5
R B;1!17-2;VALUE .5
";
        let spec = parse_e_content(content, 600).unwrap();
        assert_eq!(spec.directive().table_ids, vec![601, 603, 604, 605, 606, 608, 609, 610]);
        assert_eq!(spec.tables.len(), 8);
    }

    #[test]
    fn parse_unweight_then_weight() {
        let content = "\
TABLE 600
X ENTER NOADD
X WEIGHT UNWEIGHT
X WEIGHT 601 TH 609 TO 1!5060:5066 4
*
TABLE 601
R A;1!10-1;VALUE .5
R B;1!10-2;VALUE .5
*
TABLE 602
R A;1!11-1;VALUE .5
R B;1!11-2;VALUE .5
*
TABLE 603
R A;1!12-1;VALUE .5
R B;1!12-2;VALUE .5
*
TABLE 604
R A;1!13-1;VALUE .5
R B;1!13-2;VALUE .5
*
TABLE 605
R A;1!14-1;VALUE .5
R B;1!14-2;VALUE .5
*
TABLE 606
R A;1!15-1;VALUE .5
R B;1!15-2;VALUE .5
*
TABLE 607
R A;1!16-1;VALUE .5
R B;1!16-2;VALUE .5
*
TABLE 608
R A;1!17-1;VALUE .5
R B;1!17-2;VALUE .5
*
TABLE 609
R A;1!18-1;VALUE .5
R B;1!18-2;VALUE .5
";
        let spec = parse_e_content(content, 600).unwrap();
        assert_eq!(spec.directive().table_ids, vec![601, 602, 603, 604, 605, 606, 607, 608, 609]);
        assert_eq!(spec.tables.len(), 9);
    }

    #[test]
    fn conditions_evaluate_correctly() {
        let spec = parse_e_content(SAMPLE_E, 600).unwrap();
        // Build a record with code 1 at position 43 (index 42)
        let mut record = vec![b' '; 50];
        record[42] = b'1';
        let record = String::from_utf8(record).unwrap();

        let cat = &spec.tables[0].categories[0]; // MALE: 1!43-1
        if let WeightCondition::Uncle(ref expr) = cat.condition {
            assert!(expr.evaluate(&record).unwrap());
        } else {
            panic!("expected Uncle condition");
        }
    }

    #[test]
    fn parse_split_sample_with_x_set_qual() {
        let content = "\
TABLE 600
X ENTER NOADD
X SET QUAL(1!87-1)
X WEIGHT 601 TH 602 TO 1!2000:2006 4 OFF TOTAL 2000
X SET QUAL(1!87-2)
X WEIGHT 601 TH 602 TO 1!2000:2006 4 OFF TOTAL 2000
*
TABLE 601
R MALE;1!43-1;VALUE .48
R FEMALE;1!43-2;VALUE .52
*
TABLE 602
R YOUNG;1!41-1:3;VALUE .47
R OLD;1!41-4:7;VALUE .53
";
        let spec = parse_e_content(content, 600).unwrap();
        assert_eq!(spec.passes.len(), 2);

        // First pass: qualifier 1!87-1
        assert!(spec.passes[0].qualifier.is_some());
        let q0 = spec.passes[0].qualifier.as_ref().unwrap();
        let mut record_a = vec![b' '; 90];
        record_a[86] = b'1'; // col 87 = '1'
        assert!(q0.evaluate(&String::from_utf8(record_a).unwrap()).unwrap());

        // Second pass: qualifier 1!87-2
        assert!(spec.passes[1].qualifier.is_some());
        let q1 = spec.passes[1].qualifier.as_ref().unwrap();
        let mut record_b = vec![b' '; 90];
        record_b[86] = b'2'; // col 87 = '2'
        assert!(q1.evaluate(&String::from_utf8(record_b).unwrap()).unwrap());

        // Both passes share the same tables and directive shape.
        assert_eq!(spec.passes[0].directive.table_ids, vec![601, 602]);
        assert_eq!(spec.passes[1].directive.table_ids, vec![601, 602]);
        assert_eq!(spec.passes[0].directive.total, Some(2000.0));
        assert_eq!(spec.tables.len(), 2); // deduplicated
    }

    #[test]
    fn parse_qual_cleared_by_empty_parens() {
        let content = "\
TABLE 600
X SET QUAL(1!87-1)
X WEIGHT 601 TO 1!100:106 4 OFF
X SET QUAL()
X WEIGHT 601 TO 1!100:106 4 OFF
*
TABLE 601
R A;1!10-1;VALUE .5
R B;1!10-2;VALUE .5
";
        let spec = parse_e_content(content, 600).unwrap();
        assert_eq!(spec.passes.len(), 2);
        assert!(spec.passes[0].qualifier.is_some());
        assert!(spec.passes[1].qualifier.is_none());
    }

    #[test]
    fn parse_qual_all_clears_qualifier() {
        let content = "\
TABLE 620
X WEIGHT UNWEIGHT
X IF(1!70N1) 1!2316=1
X SET QUAL(1!70-1)
X WEIGHT 621 TO 1!2410:2416 4 OFF TOTAL 3200
X SET QUAL(ALL)
*
TABLE 621
R A;1!10-1;VALUE .5
R B;1!10-2;VALUE .5
";
        let spec = parse_e_content(content, 620).unwrap();
        assert_eq!(spec.passes.len(), 1);
        assert!(spec.passes[0].qualifier.is_some());
        assert_eq!(spec.passes[0].directive.total, Some(3200.0));

        // X IF directive should be captured as a column assignment.
        assert_eq!(spec.assignments.len(), 1);
        assert_eq!(spec.assignments[0].col, 2316);
        assert_eq!(spec.assignments[0].value, "1");
        // Condition should match records where col 70 is NOT code 1.
        let mut record_not_rv = vec![b' '; 100];
        record_not_rv[69] = b'2'; // col 70 = '2' (not registered)
        assert!(spec.assignments[0].condition.evaluate(
            &String::from_utf8(record_not_rv).unwrap()
        ).unwrap());
    }

    #[test]
    fn duplicate_table_uses_last_definition() {
        let content = "\
TABLE 600
X WEIGHT 601 TH 602 TO 1!2000:2006 4 OFF
*
TABLE 601
R MALE;1!43-1;VALUE .48
R FEMALE//OTHER;1!43-2:3;VALUE .52
*
TABLE 602
R OLD FIRST;1!41-1;VALUE .30
R OLD SECOND;1!41-2;VALUE .70
*
TABLE 603
R REGISTERED;1!70-1;VALUE .80
R NOT REGISTERED;1!70-2:3;VALUE .20
*
TABLE 602
R NEW FIRST;1!41-1;VALUE .65
R NEW SECOND;1!41-2;VALUE .35
";
        let spec = parse_e_content(content, 600).unwrap();
        assert_eq!(spec.tables.len(), 2);

        // TABLE 602 should use the second (last) definition.
        let t602 = spec.tables.iter().find(|t| t.id == 602).unwrap();
        assert_eq!(t602.categories.len(), 2);
        assert_eq!(t602.categories[0].label, "NEW FIRST");
        assert!((t602.categories[0].target - 0.65).abs() < 1e-9);
        assert_eq!(t602.categories[1].label, "NEW SECOND");
        assert!((t602.categories[1].target - 0.35).abs() < 1e-9);
    }

    #[test]
    fn parse_split_sample_different_tables() {
        let content = "\
TABLE 600
X SET QUAL(1!87-1)
X WEIGHT 601 TO 1!100:106 4 OFF TOTAL 500
X SET QUAL(1!87-2)
X WEIGHT 602 TO 1!100:106 4 OFF TOTAL 500
*
TABLE 601
R A;1!10-1;VALUE .5
R B;1!10-2;VALUE .5
*
TABLE 602
R X;1!11-1;VALUE .6
R Y;1!11-2;VALUE .4
";
        let spec = parse_e_content(content, 600).unwrap();
        assert_eq!(spec.passes.len(), 2);
        assert_eq!(spec.passes[0].directive.table_ids, vec![601]);
        assert_eq!(spec.passes[1].directive.table_ids, vec![602]);
        // Tables deduplicated from union: 601, 602
        assert_eq!(spec.tables.len(), 2);
        assert_eq!(spec.tables[0].id, 601);
        assert_eq!(spec.tables[1].id, 602);
    }
}
