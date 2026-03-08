/// UNCLE qualification syntax parser and evaluator.
///
/// Supports the subset of UNCLE syntax used in ABA weight tables 601–610,
/// plus blank (`$`) checking which is common in production weight schemes.
///
/// # Supported syntax
///
/// | Example              | Meaning                                          |
/// |----------------------|--------------------------------------------------|
/// | `1!43-1`             | Position 43 has code 1 (`1!` record prefix stripped) |
/// | `43-2:3`             | Position 43 has code 2 or 3 (code range)         |
/// | `43-4,5,7`           | Position 43 has code 4, 5, or 7 (code list)      |
/// | `43N1`               | Position 43 does NOT have code 1                 |
/// | `43-$`               | Position 43 is blank                             |
/// | `43-6:10`            | Codes 6,7,8,9,0 (10 is alias for 0 at range end) |
/// | `65,67-1`            | Position 65 or 67 has code 1 (position list)     |
/// | `65:67-1`            | Position 65, 66, or 67 has code 1 (position range)|
/// | `48-6 51-1`          | Position 48 has 6 AND position 51 has 1 (implicit AND) |
/// | `48-6 AND 51-1`      | Same, explicit AND                               |
/// | `35-1 OR 36-2`       | Either condition                                 |
/// | `NOT(43-1)`          | Logical NOT                                      |
/// | `R(26:27,9,23,25)`   | Positions 26–27 as a 2-digit numeric field = 9, 23, or 25 |
/// | `R(45:46,02)`        | Field = 2 (right-aligned, leading zeros ignored) |
/// | `R(61:62,35:44)`     | Field in range 35–44                             |
/// | `1949 'R'`           | Positions 1949 has literal char 'R'              |
use std::fmt;

// ---------------------------------------------------------------------------
// AST
// ---------------------------------------------------------------------------

/// A parsed UNCLE qualification expression.
#[derive(Debug, Clone)]
pub enum UncleExpr {
    /// Basic position test: `pos relation code_spec`.
    Term(Term),
    /// `R()` numeric field qualifier.
    RQual(RQual),
    /// Literal string match: `pos 'STRING'`.
    Literal(LiteralMatch),
    /// Logical NOT.
    Not(Box<Self>),
    /// Logical AND.
    And(Box<Self>, Box<Self>),
    /// Logical OR.
    Or(Box<Self>, Box<Self>),
}

/// The relation between a position and its code(s).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Relation {
    /// `-` : position includes this code (other codes may coexist in multi-punch data).
    Has,
    /// `N` : position does NOT include this code.
    NotHas,
    /// `=` : position has ONLY these codes.
    /// In single-punch data this is equivalent to `Has`.
    Exclusive,
}

/// A single code value in a basic term.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodeVal {
    /// Numeric code 0–9, 11, or 12 (10 is an alias for 0).
    Numeric(u8),
    /// `$` — blank position.
    Blank,
}

/// A basic position-code test.
#[derive(Debug, Clone)]
pub struct Term {
    /// 1-indexed column in the data record.
    pub col: usize,
    pub relation: Relation,
    /// The set of codes: OR-of-codes for `Has`/`NotHas`, AND-of-codes for `Exclusive`.
    pub codes: Vec<CodeVal>,
}

/// `R()` numeric field qualifier.
///
/// Reads one or more contiguous position ranges as a multi-digit number and
/// checks whether that number matches the target value(s). Multiple field groups
/// separated by `/` create an OR across groups.
#[derive(Debug, Clone)]
pub struct RQual {
    /// Each inner `Vec<usize>` is a list of consecutive 1-indexed columns forming one field.
    /// The qualifier is true if ANY field group matches.
    pub fields: Vec<Vec<usize>>,
    /// Target values: true if the field integer equals any of these.
    pub values: Vec<RValue>,
}

/// A single target value for an [`RQual`].
#[derive(Debug, Clone)]
pub enum RValue {
    Single(i64),
    Range(i64, i64),
}

/// Literal string match starting at a given column.
#[derive(Debug, Clone)]
pub struct LiteralMatch {
    /// 1-indexed column where the match begins.
    pub start_col: usize,
    /// The literal string to match (case-sensitive).
    pub value: String,
}

// ---------------------------------------------------------------------------
// Parse error
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ParseError {
    pub input: String,
    pub position: usize,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "parse error at position {}: {} (in {:?})",
            self.position, self.message, self.input
        )
    }
}

impl std::error::Error for ParseError {}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

impl UncleExpr {
    /// Parse a UNCLE qualification string into an expression tree.
    ///
    /// # Errors
    /// Returns a [`ParseError`] if the input is syntactically invalid.
    pub fn parse(input: &str) -> Result<Self, ParseError> {
        let mut p = Parser::new(input);
        let expr = p.parse_expr().map_err(|message| ParseError {
            input: input.to_owned(),
            position: p.pos,
            message,
        })?;
        p.skip_whitespace();
        if p.pos < p.chars.len() {
            let trailing: String = p.chars[p.pos..].iter().collect();
            return Err(ParseError {
                input: input.to_owned(),
                position: p.pos,
                message: format!("unexpected trailing input: {trailing:?}"),
            });
        }
        Ok(expr)
    }

    /// Evaluate the expression against a fixed-width data record (1-indexed columns).
    ///
    /// Returns `Ok(true)` if the qualification is satisfied.
    ///
    /// # Errors
    /// Returns an error string if evaluation encounters an internal inconsistency.
    pub fn evaluate(&self, record: &str) -> Result<bool, String> {
        match self {
            Self::Term(t) => t.evaluate(record),
            Self::RQual(r) => r.evaluate(record),
            Self::Literal(l) => Ok(l.evaluate(record)),
            Self::Not(inner) => Ok(!inner.evaluate(record)?),
            Self::And(l, r) => Ok(l.evaluate(record)? && r.evaluate(record)?),
            Self::Or(l, r) => Ok(l.evaluate(record)? || r.evaluate(record)?),
        }
    }
}

// ---------------------------------------------------------------------------
// Evaluation helpers
// ---------------------------------------------------------------------------

impl CodeVal {
    /// Returns true if the raw byte from the record matches this code value.
    const fn matches_byte(&self, b: u8) -> bool {
        match self {
            Self::Blank => b == b' ',
            Self::Numeric(n) => {
                let code: u8 = match b {
                    b'0'..=b'9' => b - b'0',
                    b'X' | b'-' => 11,
                    b'Y' | b'&' => 12,
                    _ => return false,
                };
                code == *n
            }
        }
    }
}

impl Term {
    fn evaluate(&self, record: &str) -> Result<bool, String> {
        let idx = self.col - 1;
        // Out-of-range columns evaluate to blank/absent.
        let byte = record.as_bytes().get(idx).copied().unwrap_or(b' ');

        match self.relation {
            Relation::Has => Ok(self.codes.iter().any(|c| c.matches_byte(byte))),
            Relation::NotHas => Ok(!self.codes.iter().any(|c| c.matches_byte(byte))),
            // For single-punch data, Exclusive is equivalent to Has.
            Relation::Exclusive => Ok(self.codes.iter().any(|c| c.matches_byte(byte))),
        }
    }
}

impl RQual {
    fn evaluate(&self, record: &str) -> Result<bool, String> {
        for field_cols in &self.fields {
            // Concatenate the characters at the specified columns.
            let field_str: String = field_cols
                .iter()
                .filter_map(|&col| record.as_bytes().get(col - 1).map(|&b| b as char))
                .collect();

            // The field is right-aligned: trim leading whitespace/zeros and parse as integer.
            let trimmed = field_str.trim_start_matches([' ', '0']);
            let field_val: i64 = if trimmed.is_empty() {
                0 // all spaces or all zeros
            } else {
                match trimmed.parse() {
                    Ok(n) => n,
                    Err(_) => continue, // non-numeric field — skip this group
                }
            };

            if self.values.iter().any(|v| v.contains(field_val)) {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

impl RValue {
    const fn contains(&self, n: i64) -> bool {
        match self {
            Self::Single(v) => *v == n,
            Self::Range(lo, hi) => n >= *lo && n <= *hi,
        }
    }
}

impl LiteralMatch {
    fn evaluate(&self, record: &str) -> bool {
        let start = self.start_col - 1;
        let end = start + self.value.len();
        record.get(start..end).is_some_and(|s| s == self.value)
    }
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

struct Parser {
    chars: Vec<char>,
    pos: usize,
}

impl Parser {
    fn new(input: &str) -> Self {
        Self { chars: input.chars().collect(), pos: 0 }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<char> {
        self.chars.get(self.pos + offset).copied()
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(c) if c.is_ascii_whitespace()) {
            self.pos += 1;
        }
    }

    /// Consume a keyword (case-insensitive) only if it is followed by a word boundary
    /// (whitespace, `(`, or end of input — not an alphanumeric character).
    fn try_keyword(&mut self, kw: &str) -> bool {
        let n = kw.len();
        if self.pos + n > self.chars.len() {
            return false;
        }
        let matches = kw.chars().zip(self.chars[self.pos..].iter()).all(|(k, &c)| {
            k.eq_ignore_ascii_case(&c)
        });
        if !matches {
            return false;
        }
        // Word boundary: next char must not be alphanumeric.
        if self.chars.get(self.pos + n).is_some_and(|c| c.is_alphanumeric()) {
            return false;
        }
        self.pos += n;
        true
    }

    fn consume_char(&mut self, expected: char) -> Result<(), String> {
        match self.peek() {
            Some(c) if c == expected => {
                self.pos += 1;
                Ok(())
            }
            Some(c) => Err(format!("expected {expected:?}, got {c:?}")),
            None => Err(format!("expected {expected:?}, got end of input")),
        }
    }

    /// Parse a sequence of ASCII digits as a `usize`.
    fn parse_usize(&mut self) -> Result<usize, String> {
        let start = self.pos;
        while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
            self.pos += 1;
        }
        if self.pos == start {
            return Err(format!("expected a number, got {:?}", self.peek()));
        }
        let s: String = self.chars[start..self.pos].iter().collect();
        s.parse::<usize>().map_err(|e| format!("invalid number {s:?}: {e}"))
    }

    // ---- Recursive-descent expression parser --------------------------------

    fn parse_expr(&mut self) -> Result<UncleExpr, String> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<UncleExpr, String> {
        let mut left = self.parse_and()?;
        loop {
            let saved = self.pos;
            self.skip_whitespace();
            if self.try_keyword("OR") {
                self.skip_whitespace();
                let right = self.parse_and()?;
                left = UncleExpr::Or(Box::new(left), Box::new(right));
            } else {
                self.pos = saved;
                break;
            }
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<UncleExpr, String> {
        let mut left = self.parse_unary()?;
        loop {
            let saved = self.pos;
            self.skip_whitespace();

            // Stop at end of input or closing paren.
            if self.pos >= self.chars.len() || self.peek() == Some(')') {
                self.pos = saved;
                break;
            }

            // Stop if the next token is OR (lower precedence).
            {
                let saved2 = self.pos;
                if self.try_keyword("OR") {
                    self.pos = saved;
                    break;
                }
                self.pos = saved2;
            }

            // Consume optional explicit AND.
            let explicit_and = {
                let saved2 = self.pos;
                if self.try_keyword("AND") {
                    self.skip_whitespace();
                    true
                } else {
                    self.pos = saved2;
                    false
                }
            };

            // If no explicit AND, the next character must be able to start a term.
            if !explicit_and && !self.can_start_term() {
                self.pos = saved;
                break;
            }

            let right = self.parse_unary()?;
            left = UncleExpr::And(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    /// Returns true if the current position can begin a unary/primary expression.
    fn can_start_term(&self) -> bool {
        match self.peek() {
            Some(c) if c.is_ascii_digit() => true,
            Some('(') => true,
            Some('R') => self.peek_at(1) == Some('('),
            Some('N') => {
                // NOT keyword
                self.chars[self.pos..].iter().collect::<String>().to_uppercase().starts_with("NOT")
                    && self.chars.get(self.pos + 3).is_none_or(|c| !c.is_alphanumeric())
            }
            _ => false,
        }
    }

    fn parse_unary(&mut self) -> Result<UncleExpr, String> {
        self.skip_whitespace();
        let saved = self.pos;
        if self.try_keyword("NOT") {
            self.skip_whitespace();
            let has_paren = self.peek() == Some('(');
            if has_paren {
                self.pos += 1; // consume '('
            }
            self.skip_whitespace();
            // Parse inner as a full OR expression so NOT(...AND...) works.
            let inner = self.parse_or()?;
            if has_paren {
                self.skip_whitespace();
                self.consume_char(')')?;
            }
            return Ok(UncleExpr::Not(Box::new(inner)));
        }
        self.pos = saved;
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<UncleExpr, String> {
        self.skip_whitespace();

        if self.peek() == Some('(') {
            self.pos += 1;
            let inner = self.parse_or()?;
            self.skip_whitespace();
            self.consume_char(')')?;
            return Ok(inner);
        }

        if self.peek() == Some('R') && self.peek_at(1) == Some('(') {
            self.pos += 2; // consume "R("
            return self.parse_r_body();
        }

        self.parse_basic_term()
    }

    // ---- R() qualifier ------------------------------------------------------

    fn parse_r_body(&mut self) -> Result<UncleExpr, String> {
        let mut fields: Vec<Vec<usize>> = Vec::new();

        loop {
            // Strip optional "1!" record prefix.
            if self.peek() == Some('1') && self.peek_at(1) == Some('!') {
                self.pos += 2;
            }
            let start_col = self.parse_usize()?;
            let end_col = if self.peek() == Some(':') {
                self.pos += 1;
                self.parse_usize()?
            } else {
                start_col
            };
            fields.push((start_col..=end_col).collect());

            match self.peek() {
                Some('/') => self.pos += 1, // next field group — continue loop

                // Ellipsis: `field1/field2...fieldN` shorthand for a regularly-spaced
                // series of same-width fields. Step and width are inferred from the
                // last two parsed fields.
                Some('.') if self.peek_at(1) == Some('.') && self.peek_at(2) == Some('.') => {
                    self.pos += 3; // consume "..."

                    // We need at least two fields to infer the step.
                    if fields.len() < 2 {
                        return Err(
                            "R() ellipsis requires at least two preceding field groups".to_string()
                        );
                    }
                    let prev = fields[fields.len() - 1].clone();
                    let prev_prev = &fields[fields.len() - 2];

                    let step = prev[0].checked_sub(prev_prev[0]).ok_or_else(|| {
                        "R() ellipsis: field groups must be in ascending order".to_string()
                    })?;
                    let width = prev.len();

                    if step == 0 {
                        return Err("R() ellipsis: step between field groups must be > 0".to_string());
                    }

                    // Strip optional "1!" before the terminal field.
                    if self.peek() == Some('1') && self.peek_at(1) == Some('!') {
                        self.pos += 2;
                    }
                    let final_start = self.parse_usize()?;
                    if self.peek() == Some(':') {
                        self.pos += 1;
                        self.parse_usize()?; // consume the end col (we derive it from width)
                    }

                    // Expand: prev[0]+step, prev[0]+2*step, … up to and including final_start.
                    let mut cur = prev[0] + step;
                    loop {
                        if cur > final_start {
                            return Err(format!(
                                "R() ellipsis: step {step} does not land on terminal field {final_start}"
                            ));
                        }
                        fields.push((cur..cur + width).collect());
                        if cur == final_start {
                            break;
                        }
                        cur += step;
                    }

                    // After the ellipsis expansion the next char must be ','.
                    self.consume_char(',')?;
                    break;
                }

                Some(',') => {
                    self.pos += 1;
                    break; // end of field specification
                }
                other => {
                    return Err(format!(
                        "expected ',', '/', or '...' inside R(), got {other:?}"
                    ))
                }
            }
        }

        let values = self.parse_r_values()?;
        self.skip_whitespace();
        self.consume_char(')')?;
        Ok(UncleExpr::RQual(RQual { fields, values }))
    }

    fn parse_r_values(&mut self) -> Result<Vec<RValue>, String> {
        let mut values = Vec::new();
        loop {
            self.skip_whitespace();
            let start = self.pos;
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.pos += 1;
            }
            if self.pos == start {
                break;
            }
            let s: String = self.chars[start..self.pos].iter().collect();
            let n: i64 = s.parse().map_err(|e| format!("invalid R() value {s:?}: {e}"))?;

            if self.peek() == Some(':') {
                self.pos += 1;
                let start2 = self.pos;
                while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                    self.pos += 1;
                }
                let s2: String = self.chars[start2..self.pos].iter().collect();
                let n2: i64 = s2.parse().map_err(|e| format!("invalid R() range end {s2:?}: {e}"))?;
                values.push(RValue::Range(n, n2));
            } else {
                values.push(RValue::Single(n));
            }

            if self.peek() == Some(',') {
                self.pos += 1;
            } else {
                break;
            }
        }
        if values.is_empty() {
            return Err("R() has no target values".to_string());
        }
        Ok(values)
    }

    // ---- Basic term (position + relation + codes) ---------------------------

    fn parse_basic_term(&mut self) -> Result<UncleExpr, String> {
        // Strip optional "1!" record prefix.
        if self.peek() == Some('1') && self.peek_at(1) == Some('!') {
            self.pos += 2;
        }

        // Parse position list. Commas and colons here are position separators
        // because we have not yet seen a relation character.
        let positions = self.parse_position_list()?;

        // Peek past whitespace to check for a literal-string match (`pos 'STRING'`).
        let saved = self.pos;
        self.skip_whitespace();
        if self.peek() == Some('\'') {
            let lit = self.parse_literal_string()?;
            // Position list must resolve to a single starting column for literals.
            let start_col = positions[0];
            return Ok(UncleExpr::Literal(LiteralMatch { start_col, value: lit }));
        }
        self.pos = saved;

        // Parse relation: `-`, `=`, or `N`.
        let relation = self.parse_relation()?;
        let codes = self.parse_code_spec()?;

        // Multiple positions desugar to an OR tree.
        let terms = positions.iter().map(|&col| {
            UncleExpr::Term(Term { col, relation: relation.clone(), codes: codes.clone() })
        });
        Ok(terms.reduce(|a, b| UncleExpr::Or(Box::new(a), Box::new(b))).unwrap())
    }

    /// Parse one or more positions separated by `,` (list) or `:` (range).
    /// Stops as soon as it sees a relation character (`-`, `=`, or `N` followed
    /// by a code character). At that point commas/colons belong to the code spec.
    fn parse_position_list(&mut self) -> Result<Vec<usize>, String> {
        let mut positions = Vec::new();
        loop {
            let first = self.parse_usize()?;

            if self.peek() == Some(':') {
                // Colon here — is it a position range or the start of the code spec?
                // It's a position range if after consuming `:number` we're still not
                // at a relation. Peek ahead to decide.
                if self.next_after_colon_is_position() {
                    self.pos += 1; // consume ':'
                    let last = self.parse_usize()?;
                    for p in first..=last {
                        positions.push(p);
                    }
                } else {
                    // The ':' belongs to the code spec — we must have already parsed
                    // the position (this shouldn't arise for well-formed input, but
                    // treat first as the sole position and stop).
                    positions.push(first);
                    break;
                }
            } else {
                positions.push(first);
            }

            // Continue with a comma-separated position only if there's no relation yet.
            if self.peek() == Some(',') && self.next_after_comma_is_position() {
                self.pos += 1; // consume ','
            } else {
                break;
            }
        }
        Ok(positions)
    }

    /// Look ahead: after a ':', skip digits and check if we're still not at a relation.
    /// Returns true if this ':' is a position-range separator.
    fn next_after_colon_is_position(&self) -> bool {
        let mut i = self.pos + 1; // skip ':'
        while i < self.chars.len() && self.chars[i].is_ascii_digit() {
            i += 1;
        }
        // After the number, if we're at a relation character it means the ':' was
        // a code-range separator — but that can't be because we haven't parsed the
        // relation yet. If we're at another ',', ':', ')' or whitespace, it's a
        // position range. If we're at a relation ('-', '=', 'N'), the range was valid.
        if i >= self.chars.len() {
            return false; // dangling, treat as no
        }
        // A position range is valid if after the second number we see a relation.
        self.chars_is_relation(i)
    }

    /// Look ahead: after a ',', skip digits (and optional colon-range) and check
    /// whether we're at a relation — meaning the ',' was a position separator.
    fn next_after_comma_is_position(&self) -> bool {
        let mut i = self.pos + 1; // skip ','
        while i < self.chars.len() && self.chars[i].is_ascii_digit() {
            i += 1;
        }
        if i < self.chars.len() && self.chars[i] == ':' {
            i += 1;
            while i < self.chars.len() && self.chars[i].is_ascii_digit() {
                i += 1;
            }
        }
        if i >= self.chars.len() {
            return false;
        }
        self.chars_is_relation(i)
    }

    /// Returns true if `self.chars[i]` is a relation character (`-`, `=`, or `N`
    /// followed by a code character).
    fn chars_is_relation(&self, i: usize) -> bool {
        match self.chars.get(i) {
            Some('-' | '=') => true,
            Some('N') => self
                .chars
                .get(i + 1)
                .is_some_and(|c| c.is_ascii_digit() || *c == '$' || *c == ','),
            _ => false,
        }
    }

    fn parse_relation(&mut self) -> Result<Relation, String> {
        match self.peek() {
            Some('-') => {
                self.pos += 1;
                Ok(Relation::Has)
            }
            Some('=') => {
                self.pos += 1;
                Ok(Relation::Exclusive)
            }
            Some('N') => {
                self.pos += 1;
                Ok(Relation::NotHas)
            }
            other => Err(format!("expected relation (-, =, N), got {other:?}")),
        }
    }

    /// Parse a code specification: one or more codes/ranges separated by commas.
    /// Code ranges use `:`. Code 10 is an alias for 0 (valid at the high end of a
    /// colon range: `6:10` → codes 6,7,8,9,0).
    fn parse_code_spec(&mut self) -> Result<Vec<CodeVal>, String> {
        let mut codes = Vec::new();
        loop {
            match self.peek() {
                Some('$') => {
                    self.pos += 1;
                    codes.push(CodeVal::Blank);
                }
                Some(c) if c.is_ascii_digit() => {
                    let start_raw = self.parse_usize()? as u8;

                    if self.peek() == Some(':') {
                        self.pos += 1;
                        let end_raw = self.parse_usize()? as u8;
                        // Expand the code range. Code 10 normalizes to 0 at the high end.
                        for raw in start_raw..=end_raw {
                            codes.push(CodeVal::Numeric(if raw == 10 { 0 } else { raw }));
                        }
                    } else {
                        codes.push(CodeVal::Numeric(if start_raw == 10 { 0 } else { start_raw }));
                    }
                }
                other => return Err(format!("expected code (digit or $), got {other:?}")),
            }

            if self.peek() == Some(',') {
                self.pos += 1;
            } else {
                break;
            }
        }
        if codes.is_empty() {
            return Err("empty code specification".to_string());
        }
        Ok(codes)
    }

    fn parse_literal_string(&mut self) -> Result<String, String> {
        self.consume_char('\'')?;
        let mut s = String::new();
        loop {
            match self.peek() {
                Some('\'') => {
                    self.pos += 1;
                    break;
                }
                Some(c) => {
                    self.pos += 1;
                    s.push(c);
                }
                None => return Err("unterminated literal string".to_string()),
            }
        }
        Ok(s)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // Helper: build a fixed-width record with specific values at specific columns.
    fn record(entries: &[(usize, &str)]) -> String {
        // Find the maximum column needed.
        let max_col = entries.iter().map(|(col, val)| col + val.len()).max().unwrap_or(1);
        let mut s = vec![b' '; max_col];
        for &(col, val) in entries {
            for (i, b) in val.bytes().enumerate() {
                s[col - 1 + i] = b;
            }
        }
        String::from_utf8(s).unwrap()
    }

    fn eval(expr: &str, rec: &str) -> bool {
        UncleExpr::parse(expr).unwrap().evaluate(rec).unwrap()
    }

    // -----------------------------------------------------------------------
    // Parsing smoke tests (round-trip via evaluate)
    // -----------------------------------------------------------------------

    #[test]
    fn parse_simple_term() {
        // 1!43-1 — position 43 has code 1
        let rec = record(&[(43, "1")]);
        assert!(eval("1!43-1", &rec));
        assert!(!eval("1!43-2", &rec));
    }

    #[test]
    fn parse_no_record_prefix() {
        // "43-1" and "1!43-1" are equivalent
        let rec = record(&[(43, "1")]);
        assert!(eval("43-1", &rec));
    }

    #[test]
    fn parse_single_digit_position() {
        // "6-1" same as "1!06-1" same as "1!6-1"
        let rec = record(&[(6, "1")]);
        assert!(eval("6-1", &rec));
        assert!(eval("06-1", &rec));
        assert!(eval("1!6-1", &rec));
    }

    #[test]
    fn parse_code_range() {
        // 43-2:3 — codes 2 or 3
        let rec2 = record(&[(43, "2")]);
        let rec3 = record(&[(43, "3")]);
        let rec1 = record(&[(43, "1")]);
        assert!(eval("1!43-2:3", &rec2));
        assert!(eval("1!43-2:3", &rec3));
        assert!(!eval("1!43-2:3", &rec1));
    }

    #[test]
    fn parse_code_list() {
        // 41-4,5,7
        let rec = record(&[(41, "5")]);
        assert!(eval("1!41-4,5,7", &rec));
        assert!(!eval("1!41-4,5,7", &record(&[(41, "6")])));
    }

    #[test]
    fn parse_code_10_alias() {
        // 66-6:10 means codes 6,7,8,9,0
        assert!(eval("66-6:10", &record(&[(66, "0")])));
        assert!(eval("66-6:10", &record(&[(66, "9")])));
        assert!(!eval("66-6:10", &record(&[(66, "1")])));
    }

    #[test]
    fn parse_blank() {
        // 17-$ means position 17 is blank
        let blank = record(&[(17, " ")]);
        let non_blank = record(&[(17, "1")]);
        assert!(eval("17-$", &blank));
        assert!(!eval("17-$", &non_blank));
        // 17=$ is equivalent
        assert!(eval("17=$", &blank));
    }

    #[test]
    fn parse_not_has_inline() {
        // 49N1 — position 49 does NOT have code 1
        assert!(eval("49N1", &record(&[(49, "2")])));
        assert!(!eval("49N1", &record(&[(49, "1")])));
    }

    // -----------------------------------------------------------------------
    // Boolean operators
    // -----------------------------------------------------------------------

    #[test]
    fn implicit_and() {
        // 48-6 51-1 — both must be true
        let rec = record(&[(48, "6"), (51, "1")]);
        assert!(eval("1!48-6 1!51-1", &rec));
        assert!(!eval("1!48-6 1!51-1", &record(&[(48, "6"), (51, "2")])));
    }

    #[test]
    fn explicit_and_keyword() {
        let rec = record(&[(48, "6"), (51, "1")]);
        assert!(eval("1!48-6 AND 1!51-1", &rec));
    }

    #[test]
    fn or_operator() {
        let rec_u = record(&[(1949, "U")]);
        let rec_s = record(&[(1949, "S")]);
        let rec_r = record(&[(1949, "R")]);
        assert!(eval("1!1949 'U' OR 1!1949 'S'", &rec_u));
        assert!(eval("1!1949 'U' OR 1!1949 'S'", &rec_s));
        assert!(!eval("1!1949 'U' OR 1!1949 'S'", &rec_r));
    }

    #[test]
    fn not_expression() {
        // NOT(44-1) — position 44 does not have code 1
        assert!(eval("NOT(44-1)", &record(&[(44, "2")])));
        assert!(!eval("NOT(44-1)", &record(&[(44, "1")])));
    }

    #[test]
    fn not_wrapping_and() {
        // NOT(44-2:4 R(45:46,1)) — the AND expression inside NOT
        // Position 44 has code 3 AND field 45:46 = "01" → true → NOT = false
        let rec = record(&[(44, "3"), (45, "0"), (46, "1")]);
        assert!(!eval("NOT(44-2:4 R(45:46,1))", &rec));
        // Position 44 has code 5 → inner AND is false → NOT = true
        let rec2 = record(&[(44, "5"), (45, "0"), (46, "1")]);
        assert!(eval("NOT(44-2:4 R(45:46,1))", &rec2));
    }

    #[test]
    fn and_or_precedence() {
        // "35-1 36-2 or 37-5" should parse as "(35-1 AND 36-2) OR 37-5"
        let both = record(&[(35, "1"), (36, "2"), (37, "3")]);
        let just37 = record(&[(35, "9"), (36, "9"), (37, "5")]);
        let neither = record(&[(35, "9"), (36, "9"), (37, "9")]);
        assert!(eval("35-1 36-2 OR 37-5", &both));
        assert!(eval("35-1 36-2 OR 37-5", &just37));
        assert!(!eval("35-1 36-2 OR 37-5", &neither));
    }

    // -----------------------------------------------------------------------
    // Position list and range
    // -----------------------------------------------------------------------

    #[test]
    fn position_list() {
        // 65,67-1 — position 65 or 67 has code 1
        assert!(eval("65,67-1", &record(&[(65, "1")])));
        assert!(eval("65,67-1", &record(&[(67, "1")])));
        assert!(!eval("65,67-1", &record(&[(66, "1")])));
    }

    #[test]
    fn position_range() {
        // 65:67-1 — position 65, 66, or 67 has code 1
        assert!(eval("65:67-1", &record(&[(65, "1")])));
        assert!(eval("65:67-1", &record(&[(66, "1")])));
        assert!(eval("65:67-1", &record(&[(67, "1")])));
        assert!(!eval("65:67-1", &record(&[(64, "1")])));
    }

    #[test]
    fn mixed_position_list() {
        // 51,53:55-6 — positions 51, 53, 54, 55 have a 6-code
        assert!(eval("51,53:55-6", &record(&[(51, "6")])));
        assert!(eval("51,53:55-6", &record(&[(53, "6")])));
        assert!(eval("51,53:55-6", &record(&[(54, "6")])));
        assert!(eval("51,53:55-6", &record(&[(55, "6")])));
        assert!(!eval("51,53:55-6", &record(&[(52, "6")])));
    }

    // -----------------------------------------------------------------------
    // R() qualifier
    // -----------------------------------------------------------------------

    #[test]
    fn r_qualifier_single_value() {
        // R(26:27,9) — 2-digit field positions 26-27 = 9
        let rec = record(&[(26, "0"), (27, "9")]); // field = "09" → 9
        assert!(eval("R(26:27,9)", &rec));
        assert!(!eval("R(26:27,8)", &rec));
    }

    #[test]
    fn r_qualifier_zero_padded() {
        // R(45:46,02) — field "02" should equal 2
        let rec = record(&[(45, "0"), (46, "2")]);
        assert!(eval("R(45:46,02)", &rec));
        assert!(eval("R(45:46,2)", &rec));
    }

    #[test]
    fn r_qualifier_value_list() {
        // R(26:27,9,23,25) — field in {9, 23, 25}
        let r9 = record(&[(26, "0"), (27, "9")]);
        let r23 = record(&[(26, "2"), (27, "3")]);
        let r10 = record(&[(26, "1"), (27, "0")]);
        assert!(eval("R(26:27,9,23,25)", &r9));
        assert!(eval("R(26:27,9,23,25)", &r23));
        assert!(!eval("R(26:27,9,23,25)", &r10));
    }

    #[test]
    fn r_qualifier_value_range() {
        // R(61:62,35:44) — field in [35,44]
        let r35 = record(&[(61, "3"), (62, "5")]);
        let r44 = record(&[(61, "4"), (62, "4")]);
        let r34 = record(&[(61, "3"), (62, "4")]);
        let r45 = record(&[(61, "4"), (62, "5")]);
        assert!(eval("R(61:62,35:44)", &r35));
        assert!(eval("R(61:62,35:44)", &r44));
        assert!(!eval("R(61:62,35:44)", &r34));
        assert!(!eval("R(61:62,35:44)", &r45));
    }

    #[test]
    fn r_qualifier_with_record_prefix() {
        // R(1!26:27,9) — 1! is stripped
        let rec = record(&[(26, "0"), (27, "9")]);
        assert!(eval("R(1!26:27,9)", &rec));
    }

    #[test]
    fn r_qualifier_not_wrapped() {
        // NOT(R(45:46,02))
        let rec_match = record(&[(45, "0"), (46, "2")]);
        let rec_no = record(&[(45, "0"), (46, "3")]);
        assert!(!eval("NOT(R(45:46,02))", &rec_match));
        assert!(eval("NOT(R(45:46,02))", &rec_no));
    }

    // -----------------------------------------------------------------------
    // Literal string match
    // -----------------------------------------------------------------------

    #[test]
    fn literal_single_char() {
        // 1!1949 'R'
        let rec_r = record(&[(1949, "R")]);
        let rec_u = record(&[(1949, "U")]);
        assert!(eval("1!1949 'R'", &rec_r));
        assert!(!eval("1!1949 'R'", &rec_u));
    }

    #[test]
    fn literal_multi_char() {
        // 44'NJ' — pos 44='N', pos 45='J'
        let rec = record(&[(44, "NJ")]);
        assert!(eval("44'NJ'", &rec));
        assert!(!eval("44'NJ'", &record(&[(44, "NX")])));
    }

    #[test]
    fn literal_case_sensitive() {
        // Literal match is case-sensitive
        assert!(!eval("44'nj'", &record(&[(44, "NJ")])));
    }

    // -----------------------------------------------------------------------
    // ABA weight table integration scenarios
    // -----------------------------------------------------------------------

    #[test]
    fn aba_table_601_gender() {
        let male = record(&[(43, "1")]);
        let female = record(&[(43, "2")]);
        let other = record(&[(43, "3")]);
        assert!(eval("1!43-1", &male));
        assert!(!eval("1!43-1", &female));
        assert!(eval("1!43-2:3", &female));
        assert!(eval("1!43-2:3", &other));
    }

    #[test]
    fn aba_table_603_age() {
        let young = record(&[(41, "2")]); // 25-34
        let senior = record(&[(41, "6")]); // 65+
        let mid = record(&[(41, "4")]); // 45-54
        assert!(eval("1!41-1:3", &young));
        assert!(!eval("1!41-1:3", &mid));
        assert!(eval("1!41-6", &senior));
        assert!(eval("1!41-4,5,7", &mid));
    }

    #[test]
    fn aba_table_604_race_white() {
        // NON-HISPANIC WHITE: 1!44-2:4 R(1!45:46,1)
        // QD7A (col 44) in 2:4 AND QD7B field (cols 45:46) = 1
        let rec = record(&[(44, "2"), (45, "0"), (46, "1")]);
        assert!(eval("1!44-2:4 R(1!45:46,1)", &rec));
    }

    #[test]
    fn aba_table_606_education_sample_split() {
        // SAMPLE A GRADUATED COLLEGE: 1!48-6 1!51-1
        let rec = record(&[(48, "6"), (51, "1")]);
        assert!(eval("1!48-6 1!51-1", &rec));
        assert!(!eval("1!48-6 1!51-1", &record(&[(48, "6"), (51, "2")])));
    }

    #[test]
    fn aba_table_607_not_registered() {
        // NOT REGISTERED: 1!49N1 1!51-1
        // Position 49 does NOT have code 1, AND position 51 has code 1
        let rec = record(&[(49, "2"), (51, "1")]);
        assert!(eval("1!49N1 1!51-1", &rec));
        let registered = record(&[(49, "1"), (51, "1")]);
        assert!(!eval("1!49N1 1!51-1", &registered));
    }

    #[test]
    fn aba_table_610_rural_urban() {
        let rural = record(&[(1949, "R")]);
        let urban = record(&[(1949, "U")]);
        let suburban = record(&[(1949, "S")]);
        assert!(eval("1!1949 'R'", &rural));
        assert!(eval("1!1949 'U' OR 1!1949 'S'", &urban));
        assert!(eval("1!1949 'U' OR 1!1949 'S'", &suburban));
        assert!(!eval("1!1949 'U' OR 1!1949 'S'", &rural));
    }

    // -----------------------------------------------------------------------
    // R() ellipsis expansion
    // -----------------------------------------------------------------------

    #[test]
    fn r_ellipsis_expands_field_series() {
        // R(1!139:140/1!141:142...1!145:146,05)
        // Step=2, width=2 → fields: 139:140, 141:142, 143:144, 145:146
        // Value 5 → field that equals 05 matches.
        let expr = UncleExpr::parse("R(1!139:140/1!141:142...1!145:146,05)").unwrap();
        let rq = match &expr {
            UncleExpr::RQual(r) => r,
            _ => panic!("expected RQual"),
        };
        assert_eq!(rq.fields.len(), 4);
        assert_eq!(rq.fields[0], vec![139, 140]);
        assert_eq!(rq.fields[1], vec![141, 142]);
        assert_eq!(rq.fields[2], vec![143, 144]);
        assert_eq!(rq.fields[3], vec![145, 146]);
    }

    #[test]
    fn r_ellipsis_matches_any_expanded_field() {
        // Field at 143:144 = "05" → value 5 → should match
        let expr = UncleExpr::parse("R(1!139:140/1!141:142...1!145:146,05)").unwrap();
        let no_match = record(&[(139, "00"), (141, "00"), (143, "00"), (145, "00")]);
        let match_143 = record(&[(139, "00"), (141, "00"), (143, "05"), (145, "00")]);
        assert!(!expr.evaluate(&no_match).unwrap());
        assert!(expr.evaluate(&match_143).unwrap());
    }

    #[test]
    fn r_ellipsis_error_if_fewer_than_two_fields() {
        assert!(UncleExpr::parse("R(1!139:140...1!145:146,05)").is_err());
    }

    // -----------------------------------------------------------------------
    // Parse errors
    // -----------------------------------------------------------------------

    #[test]
    fn parse_error_empty_input() {
        assert!(UncleExpr::parse("").is_err());
    }

    #[test]
    fn parse_error_trailing_garbage() {
        assert!(UncleExpr::parse("43-1 !!!").is_err());
    }

    #[test]
    fn parse_error_display_non_empty() {
        let err = UncleExpr::parse("43-").unwrap_err();
        assert!(!err.to_string().is_empty());
    }
}
