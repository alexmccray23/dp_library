use std::fmt;
use std::fs;
use std::path::Path;

use crate::weight::{UncleExpr, WeightCategory, WeightCondition, WeightTable};

// ---------------------------------------------------------------------------
// Public data structures
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct WeightDirective {
    pub table_start: u16,
    pub table_end: u16,
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
    pub directive: WeightDirective,
    pub tables: Vec<WeightTable>,
}

impl fmt::Debug for ParsedWeightSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ParsedWeightSpec")
            .field("directive", &self.directive)
            .field("tables_count", &self.tables.len())
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

    // Find the control table and extract X WEIGHT directive.
    let control_range = table_blocks
        .iter()
        .find(|b| b.id == control_table_id)
        .ok_or_else(|| ParseEError {
            line_num: 0,
            message: format!("TABLE {control_table_id} not found in .E file"),
        })?;

    let directive = extract_weight_directive(&lines, control_range)?;

    // Parse each weight table in the range.
    let mut tables = Vec::new();
    for table_id in directive.table_start..=directive.table_end {
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

    Ok(ParsedWeightSpec { directive, tables })
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
            blocks.push(TableBlock { id, start, end: i });
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

fn extract_weight_directive(
    lines: &[&str],
    block: &TableBlock,
) -> Result<WeightDirective, ParseEError> {
    for (i, line) in lines.iter().enumerate().take(block.end).skip(block.start) {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("X WEIGHT") {
            if rest.is_empty() || !rest.starts_with(|c: char| c.is_ascii_whitespace()) {
                continue;
            }
            return parse_weight_line(rest.trim(), i + 1);
        }
    }
    Err(ParseEError {
        line_num: block.start + 1,
        message: format!("no X WEIGHT directive found in TABLE {}", block.id),
    })
}

/// Parse: `601 TH 610 TO 1!2000:2006 4 OFF [TOTAL 800]`
fn parse_weight_line(s: &str, line_num: usize) -> Result<WeightDirective, ParseEError> {
    let err = |msg: &str| ParseEError {
        line_num,
        message: format!("X WEIGHT parse error: {msg}"),
    };

    let tokens: Vec<&str> = s.split_whitespace().collect();
    // Minimum: <start> TH <end> TO 1!<loc> <dec> OFF  → 7 tokens
    if tokens.len() < 7 {
        return Err(err("too few tokens"));
    }

    let table_start: u16 = tokens[0].parse().map_err(|_| err("invalid start table ID"))?;

    if !tokens[1].eq_ignore_ascii_case("TH") {
        return Err(err("expected 'TH' after start table ID"));
    }

    let table_end: u16 = tokens[2].parse().map_err(|_| err("invalid end table ID"))?;

    if !tokens[3].eq_ignore_ascii_case("TO") {
        return Err(err("expected 'TO' after end table ID"));
    }

    let loc_str = tokens[4].strip_prefix("1!").unwrap_or(tokens[4]);
    let (col_start, col_end) = parse_col_range(loc_str)
        .ok_or_else(|| err("invalid column range (expected e.g. 2000:2006)"))?;

    let decimal_width: usize = tokens[5].parse().map_err(|_| err("invalid decimal width"))?;

    // tokens[6] should be "OFF" — skip it.
    // Look for optional TOTAL.
    let mut total = None;
    let mut j = 7;
    while j < tokens.len() {
        if tokens[j].eq_ignore_ascii_case("TOTAL") && j + 1 < tokens.len() {
            total = Some(
                tokens[j + 1]
                    .parse::<f64>()
                    .map_err(|_| err("invalid TOTAL value"))?,
            );
            break;
        }
        j += 1;
    }

    Ok(WeightDirective {
        table_start,
        table_end,
        col_start,
        col_end,
        decimal_width,
        total,
    })
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
        let d = &spec.directive;
        assert_eq!(d.table_start, 601);
        assert_eq!(d.table_end, 602);
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
        assert_eq!(spec.directive.total, Some(800.0));
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
}
