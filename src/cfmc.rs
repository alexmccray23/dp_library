use ahash::AHashMap;
use crate::rfl::RflQuestion;

// CFMC Logic Classes
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CfmcOperator {
    // Comparison operators
    Equal,        // = or $ or #
    NotEqual,     // <>
    Less,         // <
    Greater,      // >
    LessEqual,    // <=
    GreaterEqual, // >=

    // Logical operators
    And, // AND
    Or,  // OR
    Not, // NOT

    // Special operators
    NumItems,   // NUMITEMS
    IsBlank,    // ^^B
    IsNotBlank, // ^^NB
    Plus,       // + (substring extraction or arithmetic addition)

    // Arithmetic operators
    Multiply, // *
    Divide,   // /

    // Value construction
    Comma, // ,
    Dash,  // - (range or arithmetic subtraction)
}

#[derive(Debug, Clone)]
pub enum CfmcNode {
    // Leaf nodes
    QuestionLabel(String),
    Literal(String),

    // Internal nodes
    Binary {
        operator: CfmcOperator,
        left: Box<Self>,
        right: Box<Self>,
    },
    Unary {
        operator: CfmcOperator,
        operand: Box<Self>,
    },
    FunctionCall {
        name: String,
        args: Vec<Self>,
    },
    /// Raw column reference like [10.2] — col is 1-indexed
    ColumnRef {
        col: usize,
        width: usize,
    },
    /// Punch reference like [LABEL^1,5] or [10.2^^1,5,20]
    PunchRef {
        location: Box<Self>,
        codes: Vec<u8>,
        multi_col: bool,
        negated: bool,
    },
}

#[derive(Debug, Clone)]
pub struct CfmcLogic {
    root: CfmcNode,
}

impl CfmcLogic {
    /// Parse CFMC logic expression from string
    ///
    /// # Errors
    ///
    /// Returns an error if the expression syntax is invalid
    pub fn parse(expression: &str) -> Result<Self, String> {
        let (cleaned, warnings) = Self::check_balance(expression)?;
        for warning in warnings {
            eprintln!("warning: {warning}");
        }
        let trimmed = Self::trim_expression(&cleaned);
        let root = Self::parse_node(&trimmed)?;
        Ok(Self { root })
    }

    /// Single-pass balance check over the original expression.
    ///
    /// Extraneous closing `)` / `]` are dropped from the returned string and
    /// reported as warnings (recoverable — the parser can still produce a
    /// sensible tree). Unclosed `(` / `[` / `"` are returned as `Err` because
    /// the resulting expression is genuinely malformed and any tree we build
    /// would be guesswork.
    fn check_balance(expr: &str) -> Result<(String, Vec<String>), String> {
        let mut warnings = Vec::new();
        let mut cleaned = String::with_capacity(expr.len());
        let mut paren = 0i32;
        let mut bracket = 0i32;
        let mut in_quotes = false;
        for (i, ch) in expr.char_indices() {
            match ch {
                '"' => {
                    in_quotes = !in_quotes;
                    cleaned.push(ch);
                }
                '(' if !in_quotes => {
                    paren += 1;
                    cleaned.push(ch);
                }
                ')' if !in_quotes => {
                    if paren == 0 {
                        warnings.push(format!(
                            "Unbalanced parentheses: stray ')' at position {i} in `{expr}` (ignored)"
                        ));
                        // drop the stray char
                    } else {
                        paren -= 1;
                        cleaned.push(ch);
                    }
                }
                '[' if !in_quotes => {
                    bracket += 1;
                    cleaned.push(ch);
                }
                ']' if !in_quotes => {
                    if bracket == 0 {
                        warnings.push(format!(
                            "Unbalanced brackets: stray ']' at position {i} in `{expr}` (ignored)"
                        ));
                    } else {
                        bracket -= 1;
                        cleaned.push(ch);
                    }
                }
                _ => cleaned.push(ch),
            }
        }
        if paren > 0 {
            return Err(format!(
                "Unbalanced parentheses: {paren} unclosed '(' in `{expr}`"
            ));
        }
        if bracket > 0 {
            return Err(format!(
                "Unbalanced brackets: {bracket} unclosed '[' in `{expr}`"
            ));
        }
        if in_quotes {
            return Err(format!("Unclosed quote in `{expr}`"));
        }
        Ok((cleaned, warnings))
    }

    fn trim_expression(expr: &str) -> String {
        let mut trimmed = expr.trim().to_uppercase();

        // Remove outer parentheses if they wrap the entire expression (loop to handle multiple layers)
        loop {
            if !trimmed.starts_with('(') || !trimmed.ends_with(')') {
                break;
            }

            let mut level = 0;
            let mut can_remove = true;

            for (i, ch) in trimmed.chars().enumerate() {
                match ch {
                    '(' => level += 1,
                    ')' => {
                        level -= 1;
                        if level == 0 && i < trimmed.len() - 1 {
                            can_remove = false;
                            break;
                        }
                    }
                    _ => {}
                }
            }

            if can_remove && level == 0 {
                trimmed = trimmed[1..trimmed.len() - 1].to_string();
            } else {
                break;
            }
        }

        // Remove outer brackets if they wrap the entire expression (loop to handle multiple layers)
        // Skip if inner content contains ^ (punch ref syntax handled by parse_node)
        loop {
            if !trimmed.starts_with('[') || !trimmed.ends_with(']') {
                break;
            }

            let inner = &trimmed[1..trimmed.len() - 1];
            // Skip bracket stripping for punch refs [LOC^codes] / [LOC^^codes]
            // but NOT for blank checks [LOC^^B] / [LOC^^NB] which use operator scanning
            if let Some(caret_pos) = inner.find('^') {
                let after = if inner[caret_pos..].starts_with("^^") {
                    &inner[caret_pos + 2..]
                } else {
                    &inner[caret_pos + 1..]
                };
                if after != "B" && after != "NB" {
                    break;
                }
            }

            let mut level = 0;
            let mut can_remove = true;

            for (i, ch) in trimmed.chars().enumerate() {
                match ch {
                    '[' => level += 1,
                    ']' => {
                        level -= 1;
                        if level == 0 && i < trimmed.len() - 1 {
                            can_remove = false;
                            break;
                        }
                    }
                    _ => {}
                }
            }

            if can_remove && level == 0 {
                trimmed = trimmed[1..trimmed.len() - 1].to_string();
            } else {
                break;
            }
        }

        // Remove trailing $ if present
        if trimmed.ends_with('$') {
            trimmed.pop();
        }

        trimmed
    }

    fn parse_node(logic: &str) -> Result<CfmcNode, String> {
        // Detect bracket-wrapped special syntax before trim_expression strips brackets.
        // Only treat as a single bracketed expression if the leading '[' matches the
        // trailing ']' (i.e., the entire input is wrapped in one bracket pair).
        let raw = logic.trim().to_uppercase();
        if raw.starts_with('[') && raw.ends_with(']') && {
            let bytes = raw.as_bytes();
            let mut level = 0i32;
            let mut wraps_all = true;
            for (i, &b) in bytes.iter().enumerate() {
                match b {
                    b'[' => level += 1,
                    b']' => {
                        level -= 1;
                        if level == 0 && i < bytes.len() - 1 {
                            wraps_all = false;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            wraps_all
        } {
            let inner = &raw[1..raw.len() - 1];

            // Punch references: [LOC^^codes] or [LOC^codes]
            if let Some(result) = Self::parse_punch_ref(inner)? {
                return Ok(result);
            }

            // Raw column references: [10.2] or [10]
            if let Some((col_str, width_str)) = inner.split_once('.') {
                if let (Ok(col), Ok(width)) =
                    (col_str.parse::<usize>(), width_str.parse::<usize>())
                {
                    return Ok(CfmcNode::ColumnRef { col, width });
                }
            } else if !inner.is_empty() && inner.bytes().all(|b| b.is_ascii_digit()) {
                if let Ok(col) = inner.parse::<usize>() {
                    return Ok(CfmcNode::ColumnRef { col, width: 1 });
                }
            }
        }

        let trimmed = Self::trim_expression(logic);

        if trimmed.is_empty() {
            return Err("Empty expression".to_string());
        }

        // Find the main operator by scanning with precedence
        if let Some((op, pos)) = Self::find_main_operator(&trimmed) {
            match op {
                CfmcOperator::Not | CfmcOperator::NumItems => {
                    // Unary operators (prefix)
                    let operand_text = &trimmed[pos + Self::operator_length(&op)..];
                    let operand = Self::parse_node(operand_text)?;
                    Ok(CfmcNode::Unary {
                        operator: op,
                        operand: Box::new(operand),
                    })
                }

                CfmcOperator::IsBlank | CfmcOperator::IsNotBlank => {
                    // Unary operators (postfix)
                    // Re-wrap in brackets if operand looks like col.width (from stripped [col.width^^B])
                    let operand_text = &trimmed[0..pos];
                    let operand_input = if operand_text.contains('.')
                        && operand_text.bytes().all(|b| b.is_ascii_digit() || b == b'.')
                    {
                        format!("[{operand_text}]")
                    } else {
                        operand_text.to_string()
                    };
                    let operand = Self::parse_node(&operand_input)?;
                    Ok(CfmcNode::Unary {
                        operator: op,
                        operand: Box::new(operand),
                    })
                }
                _ => {
                    // Binary operators
                    let left_text = &trimmed[..pos];
                    let right_text = &trimmed[pos + Self::operator_length(&op)..];

                    let left = if left_text.is_empty() {
                        return Err(format!("Missing left operand for operator {op:?}"));
                    } else {
                        Self::parse_node(left_text)?
                    };

                    let right = if right_text.is_empty() {
                        return Err(format!("Missing right operand for operator {op:?}"));
                    } else {
                        Self::parse_node(right_text)?
                    };

                    Ok(CfmcNode::Binary {
                        operator: op,
                        left: Box::new(left),
                        right: Box::new(right),
                    })
                }
            }
        } else {
            // Check for function calls or implicit equality like COMP(1) -> COMP = (1)
            if let Some(paren_pos) = trimmed.find('(')
                && paren_pos > 0
                && !trimmed.chars().take(paren_pos).any(char::is_whitespace)
            {
                let left_text = &trimmed[..paren_pos];

                // Known function calls: X(), XF(), NUMRESP(), etc.
                if Self::is_known_function(left_text) {
                    let inner = &trimmed[paren_pos + 1..trimmed.len() - 1];
                    let arg = Self::parse_node(inner)?;
                    return Ok(CfmcNode::FunctionCall {
                        name: left_text.to_string(),
                        args: vec![arg],
                    });
                }

                let right_text = &trimmed[paren_pos..];

                // Detect <> negation prefix: BRAND(<>1-3) → BRAND NotEqual (1-3)
                let inner_values = &trimmed[paren_pos + 1..trimmed.len() - 1];
                if let Some(stripped) = inner_values.strip_prefix("<>") {
                    let left = Self::parse_node(left_text)?;
                    let right = Self::parse_node(stripped)?;
                    return Ok(CfmcNode::Binary {
                        operator: CfmcOperator::NotEqual,
                        left: Box::new(left),
                        right: Box::new(right),
                    });
                }

                let left = Self::parse_node(left_text)?;
                let right = Self::parse_node(right_text)?;

                return Ok(CfmcNode::Binary {
                    operator: CfmcOperator::Equal,
                    left: Box::new(left),
                    right: Box::new(right),
                });
            }

            // No operator found - this is a leaf node
            Ok(Self::parse_leaf(&trimmed))
        }
    }

    /// Scans for the lowest-precedence top-level operator. Imbalance is
    /// reported once up front by `check_balance`; here we recover by clamping
    /// negative nesting levels back to zero so the rest of the scan can still
    /// find a main operator.
    fn find_main_operator(logic: &str) -> Option<(CfmcOperator, usize)> {
        let mut level = 0i32;
        let mut bracket_level = 0i32;
        let mut in_quotes = false;
        let mut lowest_precedence = 255;
        let mut best_operator = None;
        let mut best_position = 0;

        let bytes = logic.as_bytes();
        let mut i = 0;

        while i < bytes.len() {
            match bytes[i] {
                b'"' => in_quotes = !in_quotes,
                b'(' if !in_quotes => {
                    level += 1;
                }
                b')' if !in_quotes => {
                    level -= 1;
                    if level < 0 {
                        level = 0;
                    }
                }
                b'[' if !in_quotes => {
                    bracket_level += 1;
                }
                b']' if !in_quotes => {
                    bracket_level -= 1;
                    if bracket_level < 0 {
                        bracket_level = 0;
                    }
                }
                _ => {}
            }

            if !in_quotes && level == 0 && bracket_level == 0 {
                // Check for operators at current position
                if let Some((op, len)) = Self::match_operator_at_bytes(bytes, i) {
                    let precedence = Self::operator_precedence(&op);
                    if precedence <= lowest_precedence {
                        lowest_precedence = precedence;
                        best_operator = Some(op);
                        best_position = i;
                    }
                    i += len;
                    continue;
                }
            }
            i += 1;
        }

        best_operator.map(|op| (op, best_position))
    }

    /// Check if `remaining` starts with `keyword` followed by a word boundary
    /// (non-alphanumeric byte or end of input).
    fn starts_with_keyword(remaining: &[u8], keyword: &[u8]) -> bool {
        remaining.starts_with(keyword)
            && remaining
                .get(keyword.len())
                .is_none_or(|b| !b.is_ascii_alphanumeric())
    }

    fn match_operator_at_bytes(bytes: &[u8], pos: usize) -> Option<(CfmcOperator, usize)> {
        if pos >= bytes.len() {
            return None;
        }

        let remaining = &bytes[pos..];
        // Keywords need both left and right word boundaries
        let left_boundary = pos == 0 || !bytes[pos - 1].is_ascii_alphanumeric();

        // Check longer operators first
        if left_boundary && Self::starts_with_keyword(remaining, b"NUMITEMS") {
            Some((CfmcOperator::NumItems, 8))
        } else if remaining.starts_with(b"^^NB") {
            Some((CfmcOperator::IsNotBlank, 4))
        } else if left_boundary && Self::starts_with_keyword(remaining, b"NOT") {
            Some((CfmcOperator::Not, 3))
        } else if left_boundary && Self::starts_with_keyword(remaining, b"AND") {
            Some((CfmcOperator::And, 3))
        } else if remaining.starts_with(b"^^B") {
            Some((CfmcOperator::IsBlank, 3))
        } else if remaining.starts_with(b"<=") {
            Some((CfmcOperator::LessEqual, 2))
        } else if remaining.starts_with(b">=") {
            Some((CfmcOperator::GreaterEqual, 2))
        } else if remaining.starts_with(b"<>") {
            Some((CfmcOperator::NotEqual, 2))
        } else if left_boundary && Self::starts_with_keyword(remaining, b"OR") {
            Some((CfmcOperator::Or, 2))
        } else if !remaining.is_empty() {
            match remaining[0] {
                b'<' => Some((CfmcOperator::Less, 1)),
                b'>' => Some((CfmcOperator::Greater, 1)),
                b'=' | b'$' | b'#' => Some((CfmcOperator::Equal, 1)),
                b'+' => Some((CfmcOperator::Plus, 1)),
                b'*' => Some((CfmcOperator::Multiply, 1)),
                b'/' => Some((CfmcOperator::Divide, 1)),
                b',' => Some((CfmcOperator::Comma, 1)),
                b'-' => Some((CfmcOperator::Dash, 1)),
                _ => None,
            }
        } else {
            None
        }
    }

    const fn operator_precedence(op: &CfmcOperator) -> u8 {
        match op {
            CfmcOperator::Or => 10,
            CfmcOperator::And => 20,
            CfmcOperator::Equal
            | CfmcOperator::NotEqual
            | CfmcOperator::Less
            | CfmcOperator::Greater
            | CfmcOperator::LessEqual
            | CfmcOperator::GreaterEqual => 30,
            CfmcOperator::Plus => 40,
            CfmcOperator::Multiply | CfmcOperator::Divide => 45,
            CfmcOperator::Comma => 50,
            CfmcOperator::Dash => 60,
            CfmcOperator::Not
            | CfmcOperator::NumItems
            | CfmcOperator::IsBlank
            | CfmcOperator::IsNotBlank => 70,
        }
    }

    const fn operator_length(op: &CfmcOperator) -> usize {
        match op {
            CfmcOperator::NumItems => 8,
            CfmcOperator::IsNotBlank => 4,
            CfmcOperator::And | CfmcOperator::Not | CfmcOperator::IsBlank => 3,
            CfmcOperator::Or
            | CfmcOperator::LessEqual
            | CfmcOperator::GreaterEqual
            | CfmcOperator::NotEqual => 2,
            _ => 1,
        }
    }

    fn parse_leaf(trimmed: &str) -> CfmcNode {
        // If it has quotes, treat it as a literal regardless of content
        if trimmed.starts_with('"') && trimmed.ends_with('"') {
            let cleaned = &trimmed[1..trimmed.len() - 1];
            return CfmcNode::Literal(cleaned.to_string());
        }

        // Remove parentheses if present
        let cleaned = if trimmed.starts_with('(') && trimmed.ends_with(')') {
            &trimmed[1..trimmed.len() - 1]
        } else {
            trimmed
        };

        // Check if it looks like a question label (starts with letter, contains letters/numbers)
        if cleaned.chars().next().is_some_and(char::is_alphabetic)
            && cleaned.chars().all(|c| c.is_alphanumeric() || c == '_')
        {
            CfmcNode::QuestionLabel(cleaned.to_string())
        } else {
            CfmcNode::Literal(cleaned.to_string())
        }
    }

    /// Parse punch reference from bracket inner content.
    /// Handles [LOC^^codes], [LOC^codes], [LOC^^Ncodes], [LOC^^B], [LOC^^NB]
    fn parse_punch_ref(inner: &str) -> Result<Option<CfmcNode>, String> {
        // Find ^^ (multi-col) or ^ (single-col) — check ^^ first
        let (caret_pos, multi_col) = if let Some(pos) = inner.find("^^") {
            // Skip ^^B and ^^NB — those are blank checks, not punch refs
            let after_carets = &inner[pos + 2..];
            if after_carets == "B" || after_carets == "NB" {
                return Ok(None); // Let the normal ^^B/^^NB operator handling take over
            }
            (pos, true)
        } else if let Some(pos) = inner.find('^') {
            (pos, false)
        } else {
            return Ok(None);
        };

        let loc_str = &inner[..caret_pos];
        let caret_len = if multi_col { 2 } else { 1 };
        let codes_str = &inner[caret_pos + caret_len..];

        // Parse location (label or col.width)
        let location = Self::parse_node(&format!("[{loc_str}]"))?;

        // Check for N (negated) prefix
        let (negated, codes_str) = if let Some(stripped) = codes_str.strip_prefix('N') {
            (true, stripped)
        } else {
            (false, codes_str)
        };

        let codes = Self::parse_punch_codes(codes_str)?;

        Ok(Some(CfmcNode::PunchRef {
            location: Box::new(location),
            codes,
            multi_col,
            negated,
        }))
    }

    /// Parse punch code list like "1,5,8" or "1-9" or "1.5,8"
    /// Codes: 1-9 direct, 0=10, X=11, Y=12. For multi-col, 13+ maps to next columns.
    fn parse_punch_codes(codes_str: &str) -> Result<Vec<u8>, String> {
        let mut codes = Vec::new();
        for part in codes_str.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            // Check for range (period or dash)
            if let Some((start_s, end_s)) = part.split_once('-').or_else(|| part.split_once('.')) {
                let start = Self::parse_single_punch_code(start_s.trim())?;
                let end = Self::parse_single_punch_code(end_s.trim())?;
                for code in start..=end {
                    codes.push(code);
                }
            } else {
                codes.push(Self::parse_single_punch_code(part)?);
            }
        }
        Ok(codes)
    }

    fn parse_single_punch_code(s: &str) -> Result<u8, String> {
        match s {
            "X" => Ok(11),
            "Y" => Ok(12),
            _ => s
                .parse::<u8>()
                .map(|n| if n == 0 { 10 } else { n })
                .map_err(|_| format!("Invalid punch code: {s}")),
        }
    }

    /// Map a data character to its punch code number (1-12), or None if blank
    fn char_to_punch(ch: u8) -> Option<u8> {
        match ch {
            b'1'..=b'9' => Some(ch - b'0'),
            b'0' => Some(10),
            b'X' | b'x' => Some(11),
            b'Y' | b'y' => Some(12),
            _ => None,
        }
    }

    fn is_known_function(name: &str) -> bool {
        matches!(
            name,
            "X" | "XF" | "CHECKTEXT" | "NUMRESP" | "NUMBER_OF_RESPONSES"
        )
    }

    /// Evaluate the logic expression against survey responses
    ///
    /// # Errors
    ///
    /// Returns an error if a question reference is not found in the RFL or if evaluation fails
    pub fn evaluate(
        &self,
        questions: &AHashMap<String, RflQuestion>,
        response_line: &str,
    ) -> Result<bool, String> {
        Self::evaluate_node(&self.root, questions, response_line)
    }

    fn evaluate_node(
        node: &CfmcNode,
        questions: &AHashMap<String, RflQuestion>,
        response_line: &str,
    ) -> Result<bool, String> {
        match node {
            CfmcNode::QuestionLabel(label) => {
                questions.get(label).map_or_else(
                    || Err(format!("Question {label} not found in RFL")),
                    |question| {
                        let responses = question.extract_responses(response_line);
                        // Question reference evaluates to true if any response is non-empty
                        Ok(responses.iter().any(|r| !r.trim().is_empty()))
                    },
                )
            }

            CfmcNode::ColumnRef { col, width } => {
                let start = col - 1;
                if start + width <= response_line.len() {
                    let data = &response_line[start..start + width];
                    Ok(!data.trim().is_empty())
                } else {
                    Ok(false)
                }
            }

            CfmcNode::PunchRef {
                location,
                codes,
                multi_col,
                negated,
            } => {
                let result =
                    Self::evaluate_punch_ref(location, codes, *multi_col, questions, response_line)?;
                Ok(if *negated { !result } else { result })
            }

            CfmcNode::Literal(value) => {
                // Literals evaluate to true if non-empty
                Ok(!value.trim().is_empty())
            }

            CfmcNode::Binary {
                operator,
                left,
                right,
            } => match operator {
                CfmcOperator::And => {
                    let left_val = Self::evaluate_node(left, questions, response_line)?;
                    let right_val = Self::evaluate_node(right, questions, response_line)?;
                    Ok(left_val && right_val)
                }

                CfmcOperator::Or => {
                    let left_val = Self::evaluate_node(left, questions, response_line)?;
                    let right_val = Self::evaluate_node(right, questions, response_line)?;
                    Ok(left_val || right_val)
                }

                CfmcOperator::Equal => {
                    Self::evaluate_equality(left, right, questions, response_line)
                }

                CfmcOperator::NotEqual => {
                    let val = Self::evaluate_equality(left, right, questions, response_line)?;
                    Ok(!val)
                }

                CfmcOperator::Plus => {
                    // Plus operator returns substring - this shouldn't be evaluated as boolean directly
                    Err(
                        "Plus operator should not be used as a standalone boolean expression"
                            .to_string(),
                    )
                }

                CfmcOperator::Less
                | CfmcOperator::LessEqual
                | CfmcOperator::Greater
                | CfmcOperator::GreaterEqual => {
                    Self::evaluate_inequality(left, right, questions, response_line, operator)
                }

                _ => Err(format!("Binary operator {operator:?} not yet implemented")),
            },

            CfmcNode::Unary { operator, operand } => match operator {
                CfmcOperator::Not => {
                    let val = Self::evaluate_node(operand, questions, response_line)?;
                    Ok(!val)
                }

                CfmcOperator::IsBlank => Self::evaluate_blank(operand, questions, response_line),

                CfmcOperator::IsNotBlank => {
                    Ok(!Self::evaluate_blank(operand, questions, response_line)?)
                }

                CfmcOperator::NumItems => {
                    Self::evaluate_numeric_value(node, questions, response_line)?.map_or_else(
                        || Err("NUMITEMS evaluation failed".to_string()),
                        |val| Ok(val != 0.0),
                    )
                }

                _ => Err(format!("Unary operator {operator:?} not yet implemented")),
            },

            CfmcNode::FunctionCall { .. } => {
                Self::evaluate_numeric_value(node, questions, response_line)?.map_or_else(
                    || Err(format!("Cannot evaluate function as boolean: {node:?}")),
                    |val| Ok(val != 0.0),
                )
            }
        }
    }

    fn evaluate_equality(
        left: &CfmcNode,
        right: &CfmcNode,
        questions: &AHashMap<String, RflQuestion>,
        response_line: &str,
    ) -> Result<bool, String> {
        // Try numeric evaluation first (e.g., [16.1]=1). QuestionLabel returns
        // None from evaluate_numeric_value so the response-matching path below
        // still handles `QLABEL = 1,2,3` style expressions.
        if let Some(left_val) = Self::evaluate_numeric_value(left, questions, response_line)?
            && let Some(right_val) = Self::evaluate_numeric_value(right, questions, response_line)?
        {
            return Ok((left_val - right_val).abs() < f64::EPSILON);
        }

        // Get responses from left side
        let left_responses = match left {
            CfmcNode::QuestionLabel(label) => {
                if let Some(question) = questions.get(label) {
                    question.extract_responses(response_line)
                } else {
                    return Err(format!("Question {label} not found in RFL"));
                }
            }
            CfmcNode::Binary {
                operator: CfmcOperator::Plus,
                left: plus_left,
                right: plus_right,
            } => {
                // Handle Plus operation on left side - extract substring
                Self::evaluate_substring(plus_left, plus_right, questions, response_line)?
            }
            _ => {
                return Err(
                    "Left side of equality must be a question label or plus expression".to_string(),
                );
            }
        };

        // Get expected value(s) from right side
        let expected_values = Self::get_value_list(right, questions, response_line)?;

        // Check if any response matches any expected value
        Ok(Self::responses_intersect(&left_responses, &expected_values))
    }

    fn evaluate_inequality(
        left: &CfmcNode,
        right: &CfmcNode,
        questions: &AHashMap<String, RflQuestion>,
        response_line: &str,
        operator: &CfmcOperator,
    ) -> Result<bool, String> {
        // Try numeric evaluation for function calls and numeric expressions
        if let Some(left_val) = Self::evaluate_numeric_value(left, questions, response_line)? {
            if let Some(right_val) =
                Self::evaluate_numeric_value(right, questions, response_line)?
            {
                return Ok(match operator {
                    CfmcOperator::Less => left_val < right_val,
                    CfmcOperator::LessEqual => left_val <= right_val,
                    CfmcOperator::Greater => left_val > right_val,
                    CfmcOperator::GreaterEqual => left_val >= right_val,
                    _ => false,
                });
            }
        }

        // Get responses from left side (should be a question)
        let left_responses = match left {
            CfmcNode::QuestionLabel(label) => {
                if let Some(question) = questions.get(label) {
                    question.extract_responses(response_line)
                } else {
                    return Err(format!("Question {label} not found in RFL"));
                }
            }
            CfmcNode::Binary {
                operator: CfmcOperator::Plus,
                left: plus_left,
                right: plus_right,
            } => {
                // Handle Plus operation on left side - extract substring
                Self::evaluate_substring(plus_left, plus_right, questions, response_line)?
            }
            _ => return Err("Left side of equality must be a question label".to_string()),
        };

        // Get expected value(s) from right side
        let expected_values = Self::get_value_list(right, questions, response_line)?;

        // Check if any response matches any expected value
        Ok(Self::responses_comparison(
            &left_responses,
            &expected_values,
            operator,
        ))
    }

    fn evaluate_blank(
        left: &CfmcNode,
        questions: &AHashMap<String, RflQuestion>,
        response_line: &str,
    ) -> Result<bool, String> {
        match left {
            CfmcNode::QuestionLabel(label) => {
                let question = questions
                    .get(label)
                    .ok_or_else(|| format!("Question {label} not found in RFL"))?;
                let responses = question.extract_responses(response_line);
                Ok(responses.join("").trim().is_empty())
            }
            CfmcNode::ColumnRef { col, width } => {
                let start = col - 1;
                if start + width <= response_line.len() {
                    Ok(response_line[start..start + width].trim().is_empty())
                } else {
                    Ok(true) // Out of bounds = blank
                }
            }
            _ => Err(format!("Unsupported operand for blank check: {left:?}")),
        }
    }

    /// Extract raw bytes at a location (QuestionLabel or ColumnRef)
    fn extract_location_bytes<'a>(
        location: &CfmcNode,
        questions: &AHashMap<String, RflQuestion>,
        response_line: &'a str,
    ) -> Result<&'a [u8], String> {
        match location {
            CfmcNode::QuestionLabel(label) => {
                let question = questions
                    .get(label)
                    .ok_or_else(|| format!("Question {label} not found in RFL"))?;
                let start = question.start_col - 1;
                let total_width = question.width * question.max_responses;
                if start + total_width <= response_line.len() {
                    Ok(&response_line.as_bytes()[start..start + total_width])
                } else {
                    Ok(b"")
                }
            }
            CfmcNode::ColumnRef { col, width } => {
                let start = col - 1;
                if start + width <= response_line.len() {
                    Ok(&response_line.as_bytes()[start..start + width])
                } else {
                    Ok(b"")
                }
            }
            _ => Err(format!("Invalid punch ref location: {location:?}")),
        }
    }

    fn evaluate_punch_ref(
        location: &CfmcNode,
        codes: &[u8],
        multi_col: bool,
        questions: &AHashMap<String, RflQuestion>,
        response_line: &str,
    ) -> Result<bool, String> {
        let data = Self::extract_location_bytes(location, questions, response_line)?;

        for &code in codes {
            if multi_col {
                // Multi-column: code 1-12 → column 0, 13-24 → column 1, etc.
                let col_idx = ((code - 1) / 12) as usize;
                let punch_in_col = ((code - 1) % 12) + 1;
                if col_idx < data.len() {
                    if let Some(actual_punch) = Self::char_to_punch(data[col_idx]) {
                        if actual_punch == punch_in_col {
                            return Ok(true);
                        }
                    }
                }
            } else {
                // Single-column: check first column only
                if !data.is_empty() {
                    if let Some(actual_punch) = Self::char_to_punch(data[0]) {
                        if actual_punch == code {
                            return Ok(true);
                        }
                    }
                }
            }
        }

        Ok(false)
    }

    fn evaluate_numeric_value(
        node: &CfmcNode,
        questions: &AHashMap<String, RflQuestion>,
        response_line: &str,
    ) -> Result<Option<f64>, String> {
        match node {
            CfmcNode::Literal(value) => Ok(value.trim().parse::<f64>().ok()),

            // QuestionLabel intentionally returns None — question references
            // must go through the response-matching path which handles
            // multi-response questions, ranges, and value lists correctly.
            CfmcNode::QuestionLabel(_) => Ok(None),

            CfmcNode::ColumnRef { col, width } => {
                let start = col - 1;
                if start + width <= response_line.len() {
                    let data = &response_line[start..start + width];
                    Ok(data.trim().parse::<f64>().ok())
                } else {
                    Ok(Some(0.0))
                }
            }

            CfmcNode::Unary {
                operator: CfmcOperator::NumItems,
                operand,
            } => {
                if let CfmcNode::QuestionLabel(label) = operand.as_ref() {
                    let question = questions
                        .get(label)
                        .ok_or_else(|| format!("Question {label} not found in RFL"))?;
                    let responses = question.extract_responses(response_line);
                    let count = responses.iter().filter(|r| !r.trim().is_empty()).count();
                    Ok(Some(count as f64))
                } else {
                    Err("NUMITEMS requires a question label".to_string())
                }
            }

            CfmcNode::FunctionCall { name, args } => {
                Self::evaluate_function_numeric(name, args, questions, response_line)
            }

            CfmcNode::Binary {
                operator: CfmcOperator::Plus,
                left,
                right,
            } => {
                // Distinguish substring extraction (LABEL+col.width) from arithmetic
                if let CfmcNode::Literal(spec) = right.as_ref() {
                    if let Some((c, w)) = spec.split_once('.') {
                        if c.parse::<usize>().is_ok() && w.parse::<usize>().is_ok() {
                            // Substring extraction — not numeric, handled by evaluate_substring
                            return Ok(None);
                        }
                    }
                }
                Self::evaluate_arithmetic(left, right, questions, response_line, |a, b| a + b)
            }

            CfmcNode::Binary {
                operator: CfmcOperator::Dash,
                left,
                right,
            } => Self::evaluate_arithmetic(left, right, questions, response_line, |a, b| a - b),

            CfmcNode::Binary {
                operator: CfmcOperator::Multiply,
                left,
                right,
            } => Self::evaluate_arithmetic(left, right, questions, response_line, |a, b| a * b),

            CfmcNode::Binary {
                operator: CfmcOperator::Divide,
                left,
                right,
            } => Self::evaluate_arithmetic(left, right, questions, response_line, |a, b| {
                if b.abs() < f64::EPSILON { 0.0 } else { a / b }
            }),

            _ => Ok(None),
        }
    }

    fn evaluate_arithmetic(
        left: &CfmcNode,
        right: &CfmcNode,
        questions: &AHashMap<String, RflQuestion>,
        response_line: &str,
        op: impl FnOnce(f64, f64) -> f64,
    ) -> Result<Option<f64>, String> {
        if let (Some(l), Some(r)) = (
            Self::evaluate_numeric_value(left, questions, response_line)?,
            Self::evaluate_numeric_value(right, questions, response_line)?,
        ) {
            Ok(Some(op(l, r)))
        } else {
            Ok(None)
        }
    }

    fn evaluate_function_numeric(
        name: &str,
        args: &[CfmcNode],
        questions: &AHashMap<String, RflQuestion>,
        response_line: &str,
    ) -> Result<Option<f64>, String> {
        match name {
            "X" => {
                // X(label) — return 0 for blank/non-numeric, otherwise the numeric value
                let arg = args.first().ok_or("X() requires an argument")?;
                if let CfmcNode::QuestionLabel(label) = arg {
                    let question = questions
                        .get(label)
                        .ok_or_else(|| format!("Question {label} not found in RFL"))?;
                    let responses = question.extract_responses(response_line);
                    let concatenated = responses.join("");
                    Ok(Some(
                        concatenated.trim().parse::<f64>().unwrap_or(0.0),
                    ))
                } else {
                    Err("X() requires a question label argument".to_string())
                }
            }

            "XF" => {
                // XF(subfunction) — dispatch to inner function
                let inner = args.first().ok_or("XF() requires a function argument")?;
                if let CfmcNode::FunctionCall {
                    name: inner_name,
                    args: inner_args,
                } = inner
                {
                    Self::evaluate_function_numeric(inner_name, inner_args, questions, response_line)
                } else {
                    Err(format!("XF() requires a function argument, got {inner:?}"))
                }
            }

            "NUMRESP" | "NUMBER_OF_RESPONSES" => {
                // Count non-blank response slots
                let arg = args.first().ok_or("NUMRESP() requires an argument")?;
                if let CfmcNode::QuestionLabel(label) = arg {
                    let question = questions
                        .get(label)
                        .ok_or_else(|| format!("Question {label} not found in RFL"))?;
                    let responses = question.extract_responses(response_line);
                    let count = responses.iter().filter(|r| !r.trim().is_empty()).count();
                    Ok(Some(count as f64))
                } else {
                    Err("NUMRESP() requires a question label argument".to_string())
                }
            }

            "CHECKTEXT" => {
                // CHECKTEXT(label) — character count of response (0 if blank)
                let arg = args.first().ok_or("CHECKTEXT() requires an argument")?;
                if let CfmcNode::QuestionLabel(label) = arg {
                    let question = questions
                        .get(label)
                        .ok_or_else(|| format!("Question {label} not found in RFL"))?;
                    let responses = question.extract_responses(response_line);
                    let concatenated = responses.join("");
                    let trimmed = concatenated.trim();
                    Ok(Some(if trimmed.is_empty() {
                        0.0
                    } else {
                        trimmed.len() as f64
                    }))
                } else {
                    Err("CHECKTEXT() requires a question label argument".to_string())
                }
            }

            _ => Ok(None),
        }
    }

    fn evaluate_substring(
        left: &CfmcNode,
        right: &CfmcNode,
        questions: &AHashMap<String, RflQuestion>,
        response_line: &str,
    ) -> Result<Vec<String>, String> {
        // Get responses from left side (should be a question)
        let left_responses = match left {
            CfmcNode::QuestionLabel(label) => {
                if let Some(question) = questions.get(label) {
                    question.extract_responses(response_line)
                } else {
                    return Err(format!("Question {label} not found in RFL"));
                }
            }
            _ => return Err("Left side of plus sign must be a question label".to_string()),
        };

        // Get column.width specification from right side - should be a literal like "0.1"
        let CfmcNode::Literal(spec) = right else {
            return Err(
                "Right side of plus operator must be a column.width specification".to_string(),
            );
        };

        // Parse column.width specification
        if let Some((col_str, width_str)) = spec.split_once('.') {
            let col = col_str
                .parse::<usize>()
                .map_err(|_| format!("Invalid column number: {col_str}"))?;
            let width = width_str
                .parse::<usize>()
                .map_err(|_| format!("Invalid width: {width_str}"))?;

            // Concatenate all responses from the question and extract substring
            let concatenated = left_responses.join("");
            if col + width <= concatenated.len() {
                let substring = concatenated[col..col + width].to_string();
                Ok(vec![substring])
            } else {
                // If the substring extends beyond available data, return empty result
                Ok(vec![String::new()])
            }
        } else {
            Err(format!("Invalid column.width specification: {spec}"))
        }
    }

    #[allow(clippy::only_used_in_recursion)]
    fn get_value_list(
        node: &CfmcNode,
        questions: &AHashMap<String, RflQuestion>,
        response_line: &str,
    ) -> Result<Vec<String>, String> {
        match node {
            CfmcNode::Literal(value) => Ok(vec![value.clone()]),
            CfmcNode::QuestionLabel(label) => questions.get(label).map_or_else(
                // Not found in RFL — treat as a literal value (e.g. string codes like CR01)
                || Ok(vec![label.clone()]),
                |question| Ok(question.extract_responses(response_line)),
            ),
            CfmcNode::Binary {
                operator: CfmcOperator::Comma,
                left,
                right,
            } => {
                let mut left_values = Self::get_value_list(left, questions, response_line)?;
                let mut right_values = Self::get_value_list(right, questions, response_line)?;
                left_values.append(&mut right_values);
                Ok(left_values)
            }
            CfmcNode::Binary {
                operator: CfmcOperator::Dash,
                left,
                right,
            } => {
                let left_values = Self::get_value_list(left, questions, response_line)?;
                let right_values = Self::get_value_list(right, questions, response_line)?;
                if left_values.len() == 1 && right_values.len() == 1 {
                    Ok(vec![format!("{}-{}", left_values[0], right_values[0])])
                } else {
                    Err("Range operator requires single values on both sides".to_string())
                }
            }
            CfmcNode::Binary {
                operator: CfmcOperator::Plus,
                left,
                right,
            } => {
                // Handle Plus operation - extract substring
                Self::evaluate_substring(left, right, questions, response_line)
            }
            _ => Err(format!("Cannot get value list from node type {node:?}")),
        }
    }

    fn responses_intersect(responses: &[String], expected: &[String]) -> bool {
        for response in responses {
            let response_trimmed = response.trim().to_uppercase();
            if response_trimmed.is_empty() {
                continue;
            }

            for expected_val in expected {
                let expected_trimmed = expected_val.trim().to_uppercase();
                if expected_trimmed.contains('-') {
                    // Range check
                    if let Some((start, end)) = expected_trimmed.split_once('-') {
                        let start = start.trim();
                        let end = end.trim();
                        if let (Ok(num_resp), Ok(num_start), Ok(num_end)) = (
                            response_trimmed.parse::<u32>(),
                            start.parse::<u32>(),
                            end.parse::<u32>(),
                        ) && num_resp >= num_start
                            && num_resp <= num_end
                        {
                            return true;
                        }

                        if response_trimmed.len() == start.len()
                            && response_trimmed.as_str() >= start
                            && response_trimmed.as_str() <= end
                        {
                            return true;
                        }
                    }
                } else if let (Ok(num_resp), Ok(num_exp)) = (
                    response_trimmed.parse::<u32>(),
                    expected_trimmed.parse::<u32>(),
                ) && num_resp == num_exp
                {
                    return true;
                } else if response_trimmed == expected_trimmed {
                    return true;
                }
            }
        }
        false
    }

    fn responses_comparison(
        responses: &[String],
        expected: &[String],
        operator: &CfmcOperator,
    ) -> bool {
        for response in responses {
            let response_trimmed = response.trim();
            if response_trimmed.is_empty() {
                continue;
            }
            let response_val = response_trimmed.parse::<u32>().unwrap_or_default();

            for expected_val in expected {
                let expected_trimmed = expected_val.trim().to_uppercase();
                if !expected_trimmed.is_empty() {
                    let expected_val = expected_trimmed.parse::<u32>().unwrap_or_default();
                    match operator {
                        CfmcOperator::Less => return response_val < expected_val,
                        CfmcOperator::LessEqual => return response_val <= expected_val,
                        CfmcOperator::Greater => return response_val > expected_val,
                        CfmcOperator::GreaterEqual => return response_val >= expected_val,
                        _ => (),
                    }
                }
            }
        }
        false
    }

    #[allow(clippy::only_used_in_recursion)]
    fn node_to_string(node: &CfmcNode) -> String {
        Self::node_to_string_with_context(node, None)
    }

    fn node_to_string_with_context(node: &CfmcNode, parent_op: Option<&CfmcOperator>) -> String {
        match node {
            CfmcNode::QuestionLabel(label) => label.clone(),
            CfmcNode::Literal(value) => value.clone(),
            CfmcNode::Binary {
                operator,
                left,
                right,
            } => {
                let left_str = Self::node_to_string_with_context(left, Some(operator));
                let right_str = Self::node_to_string_with_context(right, Some(operator));
                let op_str = match operator {
                    CfmcOperator::Equal => "=",
                    CfmcOperator::And => " AND ",
                    CfmcOperator::Or => " OR ",
                    CfmcOperator::Plus => "+",
                    CfmcOperator::Multiply => "*",
                    CfmcOperator::Divide => "/",
                    CfmcOperator::Comma => ",",
                    CfmcOperator::Dash => "-",
                    _ => &format!(" {operator:?} "),
                };

                let result = match operator {
                    CfmcOperator::Equal => format!("{left_str}({right_str})"),
                    CfmcOperator::NotEqual => format!("{left_str}(<>{right_str})"),
                    _ => format!("{left_str}{op_str}{right_str}"),
                };

                // Add parentheses if this node has lower precedence than its parent
                if Self::needs_parentheses_for_context(operator, parent_op) {
                    format!("({result})")
                } else {
                    result
                }
            }
            CfmcNode::Unary { operator, operand } => {
                // For unary operators, check if operand needs parentheses
                // Wrap binary operands (except Equal) in parentheses
                let needs_parens = matches!(
                    operand.as_ref(),
                    CfmcNode::Binary {
                        operator: CfmcOperator::And
                            | CfmcOperator::Or
                            | CfmcOperator::Plus
                            | CfmcOperator::Comma
                            | CfmcOperator::Dash
                            | CfmcOperator::Equal,
                        ..
                    }
                );

                let operand_str = if needs_parens {
                    // Don't pass parent context for operands that will be wrapped in parentheses
                    format!("({})", Self::node_to_string_with_context(operand, None))
                } else {
                    Self::node_to_string_with_context(operand, parent_op)
                };

                let op_str = match operator {
                    CfmcOperator::Not => "NOT",
                    CfmcOperator::NumItems => "NUMITEMS",
                    _ => &format!("{operator:?}"),
                };

                // Add space after operator if not adding parentheses
                if needs_parens {
                    format!("{op_str}{operand_str}")
                } else {
                    format!("{op_str} {operand_str}")
                }
            }
            CfmcNode::FunctionCall { name, args } => {
                let args_str: Vec<String> =
                    args.iter().map(|a| Self::node_to_string(a)).collect();
                format!("{name}({})", args_str.join(","))
            }
            CfmcNode::ColumnRef { col, width } => format!("[{col}.{width}]"),
            CfmcNode::PunchRef {
                location,
                codes,
                multi_col,
                negated,
            } => {
                let loc_str = Self::node_to_string(location);
                let sep = if *multi_col { "^^" } else { "^" };
                let neg = if *negated { "N" } else { "" };
                let codes_str: Vec<String> = codes
                    .iter()
                    .map(|c| match c {
                        10 => "0".to_string(),
                        11 => "X".to_string(),
                        12 => "Y".to_string(),
                        n => n.to_string(),
                    })
                    .collect();
                format!("[{loc_str}{sep}{neg}{codes_str}]", codes_str = codes_str.join(","))
            }
        }
    }

    const fn needs_parentheses_for_context(
        current_op: &CfmcOperator,
        parent_op: Option<&CfmcOperator>,
    ) -> bool {
        if let Some(parent) = parent_op {
            let current_precedence = Self::operator_precedence(current_op);
            let parent_precedence = Self::operator_precedence(parent);

            // Add parentheses if current operator has lower precedence than parent
            // OR has lower precedence (10) than AND (20), so OR needs parentheses when under AND
            current_precedence < parent_precedence
        } else {
            false
        }
    }
}

impl std::fmt::Display for CfmcLogic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", Self::node_to_string(&self.root))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rfl::{QuestionType, RflQuestion};

    #[test]
    fn test_parse_simple_equality() {
        let logic = CfmcLogic::parse("COMP(1)").unwrap();
        let output = logic.to_string();
        assert_eq!(output, "COMP(1)");
    }

    #[test]
    fn test_parse_and_expression() {
        let logic = CfmcLogic::parse("COMP(1) AND QB(01)").unwrap();
        let output = logic.to_string();
        assert_eq!(output, "COMP(1) AND QB(01)");
    }

    #[test]
    fn test_parse_or_expression() {
        let logic = CfmcLogic::parse("COMP(1) OR COMP(2)").unwrap();
        let output = logic.to_string();
        assert_eq!(output, "COMP(1) OR COMP(2)");
    }

    #[test]
    fn test_parse_not_expression() {
        let logic = CfmcLogic::parse("NOT COMP(3)").unwrap();
        let output = logic.to_string();
        assert_eq!(output, "NOT(COMP(3))");
    }

    #[test]
    fn test_parse_comma_values() {
        let logic = CfmcLogic::parse("COMP(1,2,3)").unwrap();
        let output = logic.to_string();
        assert_eq!(output, "COMP(1,2,3)");
    }

    #[test]
    fn test_parse_range_values() {
        let logic = CfmcLogic::parse("QB(01-56)").unwrap();
        let output = logic.to_string();
        assert_eq!(output, "QB(01-56)");
    }

    #[test]
    fn test_parse_num_expression() {
        let logic = CfmcLogic::parse("[QD1#18-110]").unwrap();
        let output = logic.to_string();
        assert_eq!(output, "QD1(18-110)");
    }

    #[test]
    fn test_parse_paren_expression() {
        let logic = CfmcLogic::parse("COMP(1) AND (QB(01) OR AGEGROUP(1-3))").unwrap();
        let output = logic.to_string();
        assert_eq!(output, "COMP(1) AND (QB(01) OR AGEGROUP(1-3))");
    }

    #[test]
    fn test_check_balance_stray_close_paren_dropped() {
        let (cleaned, warnings) = CfmcLogic::check_balance("COMP(1))").unwrap();
        assert_eq!(cleaned, "COMP(1)");
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("stray ')'"));
        assert!(warnings[0].contains("position 7"));
    }

    #[test]
    fn test_check_balance_unclosed_paren_errors() {
        let err = CfmcLogic::check_balance("COMP(1").unwrap_err();
        assert!(err.contains("1 unclosed '('"));
    }

    #[test]
    fn test_check_balance_balanced_is_silent() {
        let (cleaned, warnings) = CfmcLogic::check_balance("COMP(1) AND QB(2)").unwrap();
        assert_eq!(cleaned, "COMP(1) AND QB(2)");
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_parse_recovers_from_stray_close_paren() {
        // Stray ')' should warn but still parse and evaluate as COMP(1)
        let mut questions = AHashMap::new();
        questions.insert(
            "COMP".to_string(),
            RflQuestion {
                label: "COMP".to_string(),
                start_col: 1,
                width: 1,
                question_type: QuestionType::Fld,
                max_responses: 1,
                text_lines: Vec::new(),
                responses: AHashMap::new(),
                min_value: None,
                max_value: None,
                exceptions: Vec::new(),
            },
        );
        let logic = CfmcLogic::parse("COMP(1))").unwrap();
        let line = "1".to_owned() + &"0".repeat(100);
        assert!(logic.evaluate(&questions, &line).unwrap());
        let line = "2".to_owned() + &"0".repeat(100);
        assert!(!logic.evaluate(&questions, &line).unwrap());
    }

    #[test]
    fn test_parse_errors_on_unclosed_paren() {
        // Unclosed '(' is malformed and should error
        assert!(CfmcLogic::parse("COMP(1").is_err());
    }

    #[test]
    fn test_evaluate_simple_equality() {
        let mut questions = AHashMap::new();
        let question = RflQuestion {
            label: "COMP".to_string(),
            start_col: 1,
            width: 1,
            question_type: QuestionType::Fld,
            max_responses: 1,
            text_lines: Vec::new(),
            responses: AHashMap::new(),
            min_value: None,
            max_value: None,
            exceptions: Vec::new(),
        };
        questions.insert("COMP".to_string(), question);

        let logic = CfmcLogic::parse("COMP(1)").unwrap();

        // Test with matching response
        let response_line = "1".to_owned() + &"0".repeat(100);
        assert!(logic.evaluate(&questions, &response_line).unwrap());

        // Test with non-matching response
        let response_line = "2".to_owned() + &"0".repeat(100);
        assert!(!logic.evaluate(&questions, &response_line).unwrap());
    }

    #[test]
    fn test_evaluate_and_expression() {
        let mut questions = AHashMap::new();

        let comp_question = RflQuestion {
            label: "COMP".to_string(),
            start_col: 1,
            width: 1,
            question_type: QuestionType::Fld,
            max_responses: 1,
            text_lines: Vec::new(),
            responses: AHashMap::new(),
            min_value: None,
            max_value: None,
            exceptions: Vec::new(),
        };

        let qb_question = RflQuestion {
            label: "QB".to_string(),
            start_col: 2,
            width: 2,
            question_type: QuestionType::Fld,
            max_responses: 1,
            text_lines: Vec::new(),
            responses: AHashMap::new(),
            min_value: None,
            max_value: None,
            exceptions: Vec::new(),
        };

        questions.insert("COMP".to_string(), comp_question);
        questions.insert("QB".to_string(), qb_question);

        let logic = CfmcLogic::parse("COMP(1) AND QB(05)").unwrap();

        // Test with both conditions true
        let response_line = "105";
        assert!(logic.evaluate(&questions, response_line).unwrap());

        // Test with first condition false
        let response_line = "205";
        assert!(!logic.evaluate(&questions, response_line).unwrap());

        // Test with second condition false
        let response_line = "106";
        assert!(!logic.evaluate(&questions, response_line).unwrap());
    }

    #[test]
    fn test_evaluate_comma_values() {
        let mut questions = AHashMap::new();
        let question = RflQuestion {
            label: "COMP".to_string(),
            start_col: 1,
            width: 1,
            question_type: QuestionType::Fld,
            max_responses: 1,
            text_lines: Vec::new(),
            responses: AHashMap::new(),
            min_value: None,
            max_value: None,
            exceptions: Vec::new(),
        };
        questions.insert("COMP".to_string(), question);

        let logic = CfmcLogic::parse("COMP(1,2,3)").unwrap();

        // Test with first value
        let response_line = "1";
        assert!(logic.evaluate(&questions, response_line).unwrap());

        // Test with second value
        let response_line = "2";
        assert!(logic.evaluate(&questions, response_line).unwrap());

        // Test with third value
        let response_line = "3";
        assert!(logic.evaluate(&questions, response_line).unwrap());

        // Test with non-matching value
        let response_line = "4";
        assert!(!logic.evaluate(&questions, response_line).unwrap());
    }

    #[test]
    fn test_evaluate_range_values() {
        let mut questions = AHashMap::new();
        let question = RflQuestion {
            label: "QB".to_string(),
            start_col: 1,
            width: 2,
            question_type: QuestionType::Fld,
            max_responses: 1,
            text_lines: Vec::new(),
            responses: AHashMap::new(),
            min_value: None,
            max_value: None,
            exceptions: Vec::new(),
        };
        questions.insert("QB".to_string(), question);

        let logic = CfmcLogic::parse("QB(10-15)").unwrap();

        // Test with value in range
        let response_line = "12";
        assert!(logic.evaluate(&questions, response_line).unwrap());

        // Test with value at start of range
        let response_line = "10";
        assert!(logic.evaluate(&questions, response_line).unwrap());

        // Test with value at end of range
        let response_line = "15";
        assert!(logic.evaluate(&questions, response_line).unwrap());

        // Test with value below range
        let response_line = "09";
        assert!(!logic.evaluate(&questions, response_line).unwrap());

        // Test with value above range
        let response_line = "16";
        assert!(!logic.evaluate(&questions, response_line).unwrap());
    }

    #[test]
    fn test_parse_plus_expression() {
        let logic = CfmcLogic::parse("[Q02+0.1$]=\"A\"").unwrap();
        let output = logic.to_string();
        println!("Parsed: {output}");
        assert_eq!(output, "Q02+0.1(A)");
    }

    #[test]
    fn test_parse_plus_expression2() {
        let logic = CfmcLogic::parse("FIPSCOMB(113) AND [PREC_NO+0.4$CR01,CR02,CR03,CR04,CR05,CR06,CR07,CR08,CR09,CR10,CR11,CR12,CR13,CR14,CR15,CR16,CR17,CR18,CR19,CR20,CR21,CR22,CR23,CR24,CR25,CR26,CR27,CR28,CR29,CR30,CR31,CR32,CR33,CR34,CR35,CR36,CR37,CR38,CR39,CR40,CR41,CR42,CR43,CR44]").unwrap();
        let output = logic.to_string();
        println!("Parsed: {output}");
        assert_eq!(output, "FIPSCOMB(113) AND PREC_NO+0.4(CR01,CR02,CR03,CR04,CR05,CR06,CR07,CR08,CR09,CR10,CR11,CR12,CR13,CR14,CR15,CR16,CR17,CR18,CR19,CR20,CR21,CR22,CR23,CR24,CR25,CR26,CR27,CR28,CR29,CR30,CR31,CR32,CR33,CR34,CR35,CR36,CR37,CR38,CR39,CR40,CR41,CR42,CR43,CR44)");
    }

    #[test]
    fn test_parse_caseid() {
        let logic = CfmcLogic::parse("[CASEID#101]").unwrap();
        let output = logic.to_string();
        println!("Parsed: {output}");
        assert_eq!(output, "CASEID(101)");
    }

    #[test]
    fn test_evaluate_plus_expression() {
        let mut questions = AHashMap::new();
        let q02_question = RflQuestion {
            label: "Q02".to_string(),
            start_col: 1,
            width: 3, // Make room for 3 characters
            question_type: QuestionType::Fld,
            max_responses: 1,
            text_lines: Vec::new(),
            responses: AHashMap::new(),
            min_value: None,
            max_value: None,
            exceptions: Vec::new(),
        };
        questions.insert("Q02".to_string(), q02_question);

        let logic = CfmcLogic::parse("Q02+0.1=\"A\"").unwrap();
        println!("Parsed AST: {:#?}", logic.root);

        // Test with matching response - Q02 has "ABC", taking 1 char from position 0 should give "A"
        let response_line = "ABC";
        let result = logic.evaluate(&questions, response_line);
        println!("Q02+0.1=\"A\" with response 'ABC': {result:?}");
        assert!(result.unwrap());

        // Test with non-matching response
        let response_line = "XBC";
        let result = logic.evaluate(&questions, response_line);
        println!("Q02+0.1=\"A\" with response 'XBC': {result:?}");
        assert!(!result.unwrap());
    }

    #[test]
    fn test_caseid_parsing_with_leading_zeros() {
        let mut questions = AHashMap::new();
        let caseid_question = RflQuestion {
            label: "CASEID".to_string(),
            start_col: 1,
            width: 6,
            question_type: QuestionType::Fld,
            max_responses: 1,
            text_lines: Vec::new(),
            responses: AHashMap::new(),
            min_value: None,
            max_value: None,
            exceptions: Vec::new(),
        };
        questions.insert("CASEID".to_string(), caseid_question);

        // Test parsing CASEID with leading zeros - should match 101 regardless of leading zeros
        let logic = CfmcLogic::parse("CASEID(101)").unwrap();

        // Response with leading zeros should match
        let response_line = "000101";
        assert!(logic.evaluate(&questions, response_line).unwrap());

        // Response without leading zeros should also match
        let response_line = "101   "; // padded with spaces
        assert!(logic.evaluate(&questions, response_line).unwrap());

        // Different value should not match
        let response_line = "000102";
        assert!(!logic.evaluate(&questions, response_line).unwrap());
    }

    #[test]
    fn test_caseid_range_with_leading_zeros() {
        let mut questions = AHashMap::new();
        let caseid_question = RflQuestion {
            label: "CASEID".to_string(),
            start_col: 1,
            width: 6,
            question_type: QuestionType::Fld,
            max_responses: 1,
            text_lines: Vec::new(),
            responses: AHashMap::new(),
            min_value: None,
            max_value: None,
            exceptions: Vec::new(),
        };
        questions.insert("CASEID".to_string(), caseid_question);

        // Test range evaluation with leading zeros
        let logic = CfmcLogic::parse("CASEID(100-105)").unwrap();

        // Values in range with leading zeros should match
        let response_line = "000100";
        assert!(logic.evaluate(&questions, response_line).unwrap());

        let response_line = "000103";
        assert!(logic.evaluate(&questions, response_line).unwrap());

        let response_line = "000105";
        assert!(logic.evaluate(&questions, response_line).unwrap());

        // Values outside range should not match
        let response_line = "000099";
        assert!(!logic.evaluate(&questions, response_line).unwrap());

        let response_line = "000106";
        assert!(!logic.evaluate(&questions, response_line).unwrap());
    }

    #[test]
    fn test_numeric_vs_string_comparison() {
        let mut questions = AHashMap::new();
        let test_question = RflQuestion {
            label: "TEST".to_string(),
            start_col: 1,
            width: 4,
            question_type: QuestionType::Fld,
            max_responses: 1,
            text_lines: Vec::new(),
            responses: AHashMap::new(),
            min_value: None,
            max_value: None,
            exceptions: Vec::new(),
        };
        questions.insert("TEST".to_string(), test_question);

        // Test that numeric parsing takes precedence over string comparison
        let logic = CfmcLogic::parse("TEST(5)").unwrap();

        // "05" should match "5" numerically
        let response_line = "05  ";
        assert!(logic.evaluate(&questions, response_line).unwrap());

        // "005" should match "5" numerically
        let response_line = "0005";
        assert!(logic.evaluate(&questions, response_line).unwrap());

        // Non-numeric strings should fall back to string comparison
        let logic = CfmcLogic::parse("TEST(\"ABC\")").unwrap();

        let response_line = "ABC ";
        assert!(logic.evaluate(&questions, response_line).unwrap());

        let response_line = "XYZ ";
        assert!(!logic.evaluate(&questions, response_line).unwrap());
    }

    #[test]
    fn test_mixed_numeric_string_ranges() {
        let mut questions = AHashMap::new();
        let test_question = RflQuestion {
            label: "TEST".to_string(),
            start_col: 1,
            width: 3,
            question_type: QuestionType::Fld,
            max_responses: 1,
            text_lines: Vec::new(),
            responses: AHashMap::new(),
            min_value: None,
            max_value: None,
            exceptions: Vec::new(),
        };
        questions.insert("TEST".to_string(), test_question);

        // Test numeric range
        let logic = CfmcLogic::parse("TEST(8-12)").unwrap();

        let response_line = "010"; // Should match as 10
        assert!(logic.evaluate(&questions, response_line).unwrap());

        let response_line = "007"; // Should not match as 7 < 8
        assert!(!logic.evaluate(&questions, response_line).unwrap());

        // Test string range (alphabetical)
        let logic = CfmcLogic::parse("TEST(\"A\"-\"C\")").unwrap();

        let response_line = "B  ";
        assert!(logic.evaluate(&questions, response_line).unwrap());

        let response_line = "D  ";
        assert!(!logic.evaluate(&questions, response_line).unwrap());
    }
    #[test]
    fn test_nested_not_expression() {
        let logic = CfmcLogic::parse("SAMPLTY(1,2) AND (NOT(PHRACE(02)) AND NOT((QSHISP(1) OR PHRACE(06))) AND NOT(QSHISP(2) AND PHRACE(01)))").unwrap();
        println!("Parsed AST: {:#?}", logic.root);
        let output = logic.to_string();
        println!("Parsed: {output}");
        // The parser normalizes by removing redundant outer parentheses
        assert_eq!(output, "SAMPLTY(1,2) AND NOT(PHRACE(02)) AND NOT(QSHISP(1) OR PHRACE(06)) AND NOT(QSHISP(2) AND PHRACE(01))");
    }

    #[test]
    fn test_keyword_boundary_not_in_label() {
        // Labels containing operator keywords should not be split on them
        let logic = CfmcLogic::parse("NOTABLE(1) AND ORCHID(2)").unwrap();
        let output = logic.to_string();
        assert_eq!(output, "NOTABLE(1) AND ORCHID(2)");
    }

    #[test]
    fn test_keyword_boundary_anderson() {
        // "AND" inside "ANDERSON" must not be treated as an operator
        let logic = CfmcLogic::parse("ANDERSON(1) OR COMP(2)").unwrap();
        let output = logic.to_string();
        assert_eq!(output, "ANDERSON(1) OR COMP(2)");
    }

    #[test]
    fn test_keyword_boundary_numitems_prefix() {
        // "NUMITEMS" inside "NUMITEMSX" must not be treated as an operator
        let logic = CfmcLogic::parse("NUMITEMSX(1) AND COMP(2)").unwrap();
        let output = logic.to_string();
        assert_eq!(output, "NUMITEMSX(1) AND COMP(2)");
    }

    #[test]
    fn test_parse_numitems_comparison() {
        let logic = CfmcLogic::parse("NUMITEMS(BRAND) >= 2").unwrap();
        let output = logic.to_string();
        assert_eq!(output, "NUMITEMS BRAND GreaterEqual 2");
    }

    #[test]
    fn test_evaluate_numitems() {
        let mut questions = AHashMap::new();
        let question = RflQuestion {
            label: "BRAND".to_string(),
            start_col: 1,
            width: 1,
            question_type: QuestionType::Fld,
            max_responses: 3,
            text_lines: Vec::new(),
            responses: AHashMap::new(),
            min_value: None,
            max_value: None,
            exceptions: Vec::new(),
        };
        questions.insert("BRAND".to_string(), question);

        let logic = CfmcLogic::parse("NUMITEMS(BRAND) >= 2").unwrap();

        // 3 non-blank responses → NUMITEMS = 3 >= 2 → true
        let response_line = "123";
        assert!(logic.evaluate(&questions, response_line).unwrap());

        // 1 non-blank response → NUMITEMS = 1 >= 2 → false
        let response_line = "1  ";
        assert!(!logic.evaluate(&questions, response_line).unwrap());

        // 2 non-blank responses → NUMITEMS = 2 >= 2 → true
        let response_line = "12 ";
        assert!(logic.evaluate(&questions, response_line).unwrap());
    }

    #[test]
    fn test_parse_x_function() {
        let logic = CfmcLogic::parse("X(AGE) >= 21").unwrap();
        let output = logic.to_string();
        assert_eq!(output, "X(AGE) GreaterEqual 21");
    }

    #[test]
    fn test_evaluate_x_function() {
        let mut questions = AHashMap::new();
        let question = RflQuestion {
            label: "AGE".to_string(),
            start_col: 1,
            width: 3,
            question_type: QuestionType::Num,
            max_responses: 1,
            text_lines: Vec::new(),
            responses: AHashMap::new(),
            min_value: None,
            max_value: None,
            exceptions: Vec::new(),
        };
        questions.insert("AGE".to_string(), question);

        let logic = CfmcLogic::parse("X(AGE) >= 21").unwrap();

        // Numeric value 25 >= 21 → true
        let response_line = "025";
        assert!(logic.evaluate(&questions, response_line).unwrap());

        // Numeric value 18 >= 21 → false
        let response_line = "018";
        assert!(!logic.evaluate(&questions, response_line).unwrap());

        // Blank response → X() returns 0, 0 >= 21 → false
        let response_line = "   ";
        assert!(!logic.evaluate(&questions, response_line).unwrap());
    }

    #[test]
    fn test_parse_xf_numresp() {
        let logic = CfmcLogic::parse("XF(NUMRESP(BEV1)) > 1").unwrap();
        let output = logic.to_string();
        assert_eq!(output, "XF(NUMRESP(BEV1)) Greater 1");
    }

    #[test]
    fn test_evaluate_xf_numresp() {
        let mut questions = AHashMap::new();
        let question = RflQuestion {
            label: "BEV1".to_string(),
            start_col: 1,
            width: 2,
            question_type: QuestionType::Fld,
            max_responses: 3,
            text_lines: Vec::new(),
            responses: AHashMap::new(),
            min_value: None,
            max_value: None,
            exceptions: Vec::new(),
        };
        questions.insert("BEV1".to_string(), question);

        let logic = CfmcLogic::parse("XF(NUMRESP(BEV1)) > 1").unwrap();

        // 2 non-blank responses → count = 2 > 1 → true
        let response_line = "0102  ";
        assert!(logic.evaluate(&questions, response_line).unwrap());

        // 1 non-blank response → count = 1 > 1 → false
        let response_line = "01    ";
        assert!(!logic.evaluate(&questions, response_line).unwrap());

        // 3 non-blank responses → count = 3 > 1 → true
        let response_line = "010203";
        assert!(logic.evaluate(&questions, response_line).unwrap());
    }

    #[test]
    fn test_parse_xf_number_of_responses() {
        // Full name should also work
        let logic = CfmcLogic::parse("XF(NUMBER_OF_RESPONSES(BEV1)) > 1").unwrap();
        let output = logic.to_string();
        assert_eq!(output, "XF(NUMBER_OF_RESPONSES(BEV1)) Greater 1");
    }

    #[test]
    fn test_x_function_with_and() {
        // X() used in a compound expression
        let logic = CfmcLogic::parse("X(FEB) < X(JAN)").unwrap();
        let output = logic.to_string();
        assert_eq!(output, "X(FEB) Less X(JAN)");
    }

    #[test]
    fn test_evaluate_x_comparison() {
        let mut questions = AHashMap::new();
        for (label, col) in [("FEB", 1), ("JAN", 4)] {
            let question = RflQuestion {
                label: label.to_string(),
                start_col: col,
                width: 3,
                question_type: QuestionType::Num,
                max_responses: 1,
                text_lines: Vec::new(),
                responses: AHashMap::new(),
                min_value: None,
                max_value: None,
                exceptions: Vec::new(),
            };
            questions.insert(label.to_string(), question);
        }

        let logic = CfmcLogic::parse("X(FEB) < X(JAN)").unwrap();

        // FEB=10, JAN=20 → 10 < 20 → true
        let response_line = "010020";
        assert!(logic.evaluate(&questions, response_line).unwrap());

        // FEB=30, JAN=20 → 30 < 20 → false
        let response_line = "030020";
        assert!(!logic.evaluate(&questions, response_line).unwrap());

        // FEB=blank, JAN=20 → 0 < 20 → true
        let response_line = "   020";
        assert!(logic.evaluate(&questions, response_line).unwrap());
    }

    #[test]
    fn test_parse_checktext() {
        let logic = CfmcLogic::parse("CHECKTEXT(OTHER) > 0").unwrap();
        let output = logic.to_string();
        assert_eq!(output, "CHECKTEXT(OTHER) Greater 0");
    }

    #[test]
    fn test_evaluate_checktext() {
        let mut questions = AHashMap::new();
        let question = RflQuestion {
            label: "OTHER".to_string(),
            start_col: 1,
            width: 10,
            question_type: QuestionType::Var,
            max_responses: 1,
            text_lines: Vec::new(),
            responses: AHashMap::new(),
            min_value: None,
            max_value: None,
            exceptions: Vec::new(),
        };
        questions.insert("OTHER".to_string(), question);

        let logic = CfmcLogic::parse("CHECKTEXT(OTHER) > 0").unwrap();

        // Non-blank response → CHECKTEXT = 5 > 0 → true
        let response_line = "Hello     ";
        assert!(logic.evaluate(&questions, response_line).unwrap());

        // Blank response → CHECKTEXT = 0 > 0 → false
        let response_line = "          ";
        assert!(!logic.evaluate(&questions, response_line).unwrap());

        let logic = CfmcLogic::parse("CHECKTEXT(OTHER) > 1").unwrap();

        // Single char → CHECKTEXT = 1 > 1 → false
        let response_line = "X         ";
        assert!(!logic.evaluate(&questions, response_line).unwrap());
    }

    #[test]
    fn test_parse_negated_response_list() {
        let logic = CfmcLogic::parse("BRAND(<>1-3)").unwrap();
        let output = logic.to_string();
        // Should produce NotEqual, not Equal
        assert_eq!(output, "BRAND(<>1-3)");
    }

    #[test]
    fn test_evaluate_negated_response_list() {
        let mut questions = AHashMap::new();
        let question = RflQuestion {
            label: "BRAND".to_string(),
            start_col: 1,
            width: 1,
            question_type: QuestionType::Fld,
            max_responses: 1,
            text_lines: Vec::new(),
            responses: AHashMap::new(),
            min_value: None,
            max_value: None,
            exceptions: Vec::new(),
        };
        questions.insert("BRAND".to_string(), question);

        let logic = CfmcLogic::parse("BRAND(<>1-3)").unwrap();

        // Response 4 is NOT in 1-3 → true
        let response_line = "4";
        assert!(logic.evaluate(&questions, response_line).unwrap());

        // Response 2 IS in 1-3 → false
        let response_line = "2";
        assert!(!logic.evaluate(&questions, response_line).unwrap());

        // Response 1 IS in 1-3 → false
        let response_line = "1";
        assert!(!logic.evaluate(&questions, response_line).unwrap());
    }

    #[test]
    fn test_evaluate_negated_comma_list() {
        let mut questions = AHashMap::new();
        let question = RflQuestion {
            label: "Q1".to_string(),
            start_col: 1,
            width: 1,
            question_type: QuestionType::Fld,
            max_responses: 1,
            text_lines: Vec::new(),
            responses: AHashMap::new(),
            min_value: None,
            max_value: None,
            exceptions: Vec::new(),
        };
        questions.insert("Q1".to_string(), question);

        let logic = CfmcLogic::parse("Q1(<>2,4,6)").unwrap();

        // Response 3 not in {2,4,6} → true
        let response_line = "3";
        assert!(logic.evaluate(&questions, response_line).unwrap());

        // Response 4 in {2,4,6} → false
        let response_line = "4";
        assert!(!logic.evaluate(&questions, response_line).unwrap());
    }

    #[test]
    fn test_parse_column_ref() {
        let logic = CfmcLogic::parse("[10.2] > 9").unwrap();
        let output = logic.to_string();
        assert_eq!(output, "[10.2] Greater 9");
    }

    #[test]
    fn test_parse_column_ref_no_width() {
        // Single column, default width 1
        let logic = CfmcLogic::parse("[5] > 3").unwrap();
        let output = logic.to_string();
        assert_eq!(output, "[5.1] Greater 3");
    }

    #[test]
    fn test_evaluate_column_ref() {
        let questions = AHashMap::new();

        let logic = CfmcLogic::parse("[10.2] > 9").unwrap();

        // Columns 10-11 = "15" → 15 > 9 → true
        let response_line = "000000000150000";
        assert!(logic.evaluate(&questions, response_line).unwrap());

        // Columns 10-11 = "05" → 5 > 9 → false
        let response_line = "000000000050000";
        assert!(!logic.evaluate(&questions, response_line).unwrap());

        // Columns 10-11 = "99" → 99 > 9 → true
        let response_line = "000000000990000";
        assert!(logic.evaluate(&questions, response_line).unwrap());
    }

    #[test]
    fn test_column_ref_in_compound_expr() {
        let mut questions = AHashMap::new();
        let question = RflQuestion {
            label: "COMP".to_string(),
            start_col: 1,
            width: 1,
            question_type: QuestionType::Fld,
            max_responses: 1,
            text_lines: Vec::new(),
            responses: AHashMap::new(),
            min_value: None,
            max_value: None,
            exceptions: Vec::new(),
        };
        questions.insert("COMP".to_string(), question);

        // Mix column ref with question label
        let logic = CfmcLogic::parse("COMP(1) AND [5.2] > 20").unwrap();

        // COMP=1 and cols 5-6 = "25" → true AND 25>20 → true
        let response_line = "1   25";
        assert!(logic.evaluate(&questions, response_line).unwrap());

        // COMP=1 and cols 5-6 = "10" → true AND 10>20 → false
        let response_line = "1   10";
        assert!(!logic.evaluate(&questions, response_line).unwrap());
    }

    #[test]
    fn test_column_ref_does_not_break_label_brackets() {
        // [QD1#18-110] should still parse as a label with # range, not a ColumnRef
        let logic = CfmcLogic::parse("[QD1#18-110]").unwrap();
        let output = logic.to_string();
        assert_eq!(output, "QD1(18-110)");
    }

    #[test]
    fn test_arithmetic_multiply_divide() {
        let questions = AHashMap::new();

        // (5 * 3) > 10 → 15 > 10 → true
        let logic = CfmcLogic::parse("[1.1] * [2.1] > 10").unwrap();
        let response_line = "53";
        assert!(logic.evaluate(&questions, response_line).unwrap());

        // 8 / 2 > 5 → 4 > 5 → false
        let logic = CfmcLogic::parse("[1.1] / [2.1] > 5").unwrap();
        let response_line = "82";
        assert!(!logic.evaluate(&questions, response_line).unwrap());
    }

    #[test]
    fn test_arithmetic_precedence() {
        let questions = AHashMap::new();

        // 2 + 3 * 4 should be 2 + (3*4) = 14, not (2+3)*4 = 20
        // [1.1]=2, [2.1]=3, [3.1]=4
        let logic = CfmcLogic::parse("[1.1] + [2.1] * [3.1] > 13").unwrap();
        let response_line = "234";
        assert!(logic.evaluate(&questions, response_line).unwrap());

        // Same expression > 15 → 14 > 15 → false (proves it's 14 not 20)
        let logic = CfmcLogic::parse("[1.1] + [2.1] * [3.1] > 15").unwrap();
        assert!(!logic.evaluate(&questions, response_line).unwrap());
    }

    #[test]
    fn test_arithmetic_with_x_function() {
        let mut questions = AHashMap::new();
        let question = RflQuestion {
            label: "AGE".to_string(),
            start_col: 1,
            width: 3,
            question_type: QuestionType::Num,
            max_responses: 1,
            text_lines: Vec::new(),
            responses: AHashMap::new(),
            min_value: None,
            max_value: None,
            exceptions: Vec::new(),
        };
        questions.insert("AGE".to_string(), question);

        // (5 + [10.2]) / AGE > 23 from the PDF example
        // But using X(AGE) for safe numeric extraction
        let logic = CfmcLogic::parse("(5 + [4.2]) / X(AGE) > 1").unwrap();

        // AGE=3, [4.2]=10 → (5+10)/3 = 5 > 1 → true
        let response_line = "003105";
        assert!(logic.evaluate(&questions, response_line).unwrap());
    }

    #[test]
    fn test_arithmetic_subtraction() {
        let questions = AHashMap::new();

        // [1.2] - [3.2] > 0 → 50 - 30 = 20 > 0 → true
        let logic = CfmcLogic::parse("[1.2] - [3.2] > 0").unwrap();
        let response_line = "5030";
        assert!(logic.evaluate(&questions, response_line).unwrap());
    }

    #[test]
    fn test_substring_still_works_with_arithmetic() {
        // Ensure LABEL+0.4$ syntax still works (not treated as arithmetic)
        let logic = CfmcLogic::parse("Q02+0.1=\"A\"").unwrap();
        let output = logic.to_string();
        assert_eq!(output, "Q02+0.1(A)");
    }

    #[test]
    fn test_parse_punch_ref_single_col() {
        let logic = CfmcLogic::parse("[Q1^1,5]").unwrap();
        let output = logic.to_string();
        assert_eq!(output, "[Q1^1,5]");
    }

    #[test]
    fn test_parse_punch_ref_multi_col() {
        let logic = CfmcLogic::parse("[Q1^^1,5]").unwrap();
        let output = logic.to_string();
        assert_eq!(output, "[Q1^^1,5]");
    }

    #[test]
    fn test_parse_punch_ref_negated() {
        let logic = CfmcLogic::parse("[Q1^^N1,5]").unwrap();
        let output = logic.to_string();
        assert_eq!(output, "[Q1^^N1,5]");
    }

    #[test]
    fn test_parse_punch_ref_with_column_ref() {
        let logic = CfmcLogic::parse("[10.2^^1,3,5]").unwrap();
        let output = logic.to_string();
        assert_eq!(output, "[[10.2]^^1,3,5]");
    }

    #[test]
    fn test_evaluate_punch_ref_single_col() {
        let mut questions = AHashMap::new();
        let question = RflQuestion {
            label: "Q1".to_string(),
            start_col: 1,
            width: 1,
            question_type: QuestionType::Fld,
            max_responses: 1,
            text_lines: Vec::new(),
            responses: AHashMap::new(),
            min_value: None,
            max_value: None,
            exceptions: Vec::new(),
        };
        questions.insert("Q1".to_string(), question);

        let logic = CfmcLogic::parse("[Q1^5]").unwrap();

        // Data byte is '5' → punch code 5 → matches → true
        let response_line = "5";
        assert!(logic.evaluate(&questions, response_line).unwrap());

        // Data byte is '3' → punch code 3 → no match → false
        let response_line = "3";
        assert!(!logic.evaluate(&questions, response_line).unwrap());

        // Data byte is ' ' → no punch → false
        let response_line = " ";
        assert!(!logic.evaluate(&questions, response_line).unwrap());
    }

    #[test]
    fn test_evaluate_punch_ref_multi_col() {
        let mut questions = AHashMap::new();
        let question = RflQuestion {
            label: "Q1".to_string(),
            start_col: 1,
            width: 1,
            question_type: QuestionType::Fld,
            max_responses: 3,
            text_lines: Vec::new(),
            responses: AHashMap::new(),
            min_value: None,
            max_value: None,
            exceptions: Vec::new(),
        };
        questions.insert("Q1".to_string(), question);

        // Multi-col: code 1-12 → col 0, 13-24 → col 1, 25-36 → col 2
        // Code 1 = punch 1 in col 0
        let logic = CfmcLogic::parse("[Q1^^1]").unwrap();

        // First col has '1' → punch 1 → matches
        let response_line = "1  ";
        assert!(logic.evaluate(&questions, response_line).unwrap());

        // First col has '5' → punch 5, not 1 → no match
        let response_line = "5  ";
        assert!(!logic.evaluate(&questions, response_line).unwrap());
    }

    #[test]
    fn test_evaluate_punch_ref_negated() {
        let mut questions = AHashMap::new();
        let question = RflQuestion {
            label: "Q1".to_string(),
            start_col: 1,
            width: 1,
            question_type: QuestionType::Fld,
            max_responses: 1,
            text_lines: Vec::new(),
            responses: AHashMap::new(),
            min_value: None,
            max_value: None,
            exceptions: Vec::new(),
        };
        questions.insert("Q1".to_string(), question);

        let logic = CfmcLogic::parse("[Q1^N5]").unwrap();

        // Data '5' → punch 5 → matches → negated → false
        let response_line = "5";
        assert!(!logic.evaluate(&questions, response_line).unwrap());

        // Data '3' → punch 3, not 5 → no match → negated → true
        let response_line = "3";
        assert!(logic.evaluate(&questions, response_line).unwrap());
    }

    #[test]
    fn test_evaluate_punch_ref_special_codes() {
        let mut questions = AHashMap::new();
        let question = RflQuestion {
            label: "Q1".to_string(),
            start_col: 1,
            width: 1,
            question_type: QuestionType::Fld,
            max_responses: 1,
            text_lines: Vec::new(),
            responses: AHashMap::new(),
            min_value: None,
            max_value: None,
            exceptions: Vec::new(),
        };
        questions.insert("Q1".to_string(), question);

        // Code 0 → maps to punch 10, data '0' → punch 10
        let logic = CfmcLogic::parse("[Q1^0]").unwrap();
        let response_line = "0";
        assert!(logic.evaluate(&questions, response_line).unwrap());

        // Code X → punch 11, data 'X'
        let logic = CfmcLogic::parse("[Q1^X]").unwrap();
        let response_line = "X";
        assert!(logic.evaluate(&questions, response_line).unwrap());

        // Code Y → punch 12, data 'Y'
        let logic = CfmcLogic::parse("[Q1^Y]").unwrap();
        let response_line = "Y";
        assert!(logic.evaluate(&questions, response_line).unwrap());
    }

    #[test]
    fn test_parse_blank_check_label() {
        let logic = CfmcLogic::parse("[Q1^^B]").unwrap();
        let output = logic.to_string();
        assert_eq!(output, "IsBlank Q1");
    }

    #[test]
    fn test_parse_not_blank_check_label() {
        let logic = CfmcLogic::parse("[Q1^^NB]").unwrap();
        let output = logic.to_string();
        assert_eq!(output, "IsNotBlank Q1");
    }

    #[test]
    fn test_parse_not_blank_check_label_or() {
        let logic = CfmcLogic::parse("[Q1^^NB] OR [Q2^^NB]").unwrap();
        let output = logic.to_string();
        assert_eq!(output, "IsNotBlank Q1 OR IsNotBlank Q2");
    }

    #[test]
    fn test_parse_blank_check_label_and() {
        let logic = CfmcLogic::parse("[Q1^^B] AND [Q2^^B]").unwrap();
        let output = logic.to_string();
        assert_eq!(output, "IsBlank Q1 AND IsBlank Q2");
    }

    #[test]
    fn test_evaluate_blank_check_label() {
        let mut questions = AHashMap::new();
        let question = RflQuestion {
            label: "Q1".to_string(),
            start_col: 1,
            width: 3,
            question_type: QuestionType::Fld,
            max_responses: 1,
            text_lines: Vec::new(),
            responses: AHashMap::new(),
            min_value: None,
            max_value: None,
            exceptions: Vec::new(),
        };
        questions.insert("Q1".to_string(), question);

        let logic = CfmcLogic::parse("[Q1^^B]").unwrap();

        // Blank response → true
        let response_line = "   ";
        assert!(logic.evaluate(&questions, response_line).unwrap());

        // Non-blank response → false
        let response_line = "ABC";
        assert!(!logic.evaluate(&questions, response_line).unwrap());
    }

    #[test]
    fn test_evaluate_not_blank_check_label() {
        let mut questions = AHashMap::new();
        let question = RflQuestion {
            label: "Q1".to_string(),
            start_col: 1,
            width: 3,
            question_type: QuestionType::Fld,
            max_responses: 1,
            text_lines: Vec::new(),
            responses: AHashMap::new(),
            min_value: None,
            max_value: None,
            exceptions: Vec::new(),
        };
        questions.insert("Q1".to_string(), question);

        let logic = CfmcLogic::parse("[Q1^^NB]").unwrap();

        // Blank response → false
        let response_line = "   ";
        assert!(!logic.evaluate(&questions, response_line).unwrap());

        // Non-blank response → true
        let response_line = "ABC";
        assert!(logic.evaluate(&questions, response_line).unwrap());
    }

    #[test]
    fn test_evaluate_blank_check_column_ref() {
        let questions = AHashMap::new();

        let logic = CfmcLogic::parse("[10.2^^B]").unwrap();

        // Columns 10-11 blank → true
        let response_line = "000000000  0000";
        assert!(logic.evaluate(&questions, response_line).unwrap());

        // Columns 10-11 non-blank → false
        let response_line = "000000000150000";
        assert!(!logic.evaluate(&questions, response_line).unwrap());
    }

    #[test]
    fn test_evaluate_not_blank_check_column_ref() {
        let questions = AHashMap::new();

        let logic = CfmcLogic::parse("[10.2^^NB]").unwrap();

        // Columns 10-11 blank → false
        let response_line = "000000000  0000";
        assert!(!logic.evaluate(&questions, response_line).unwrap());

        // Columns 10-11 non-blank → true
        let response_line = "000000000150000";
        assert!(logic.evaluate(&questions, response_line).unwrap());
    }

    #[test]
    fn test_punch_ref_in_compound_expr() {
        let mut questions = AHashMap::new();
        let question = RflQuestion {
            label: "Q1".to_string(),
            start_col: 1,
            width: 1,
            question_type: QuestionType::Fld,
            max_responses: 1,
            text_lines: Vec::new(),
            responses: AHashMap::new(),
            min_value: None,
            max_value: None,
            exceptions: Vec::new(),
        };
        let comp = RflQuestion {
            label: "COMP".to_string(),
            start_col: 2,
            width: 1,
            question_type: QuestionType::Fld,
            max_responses: 1,
            text_lines: Vec::new(),
            responses: AHashMap::new(),
            min_value: None,
            max_value: None,
            exceptions: Vec::new(),
        };
        questions.insert("Q1".to_string(), question);
        questions.insert("COMP".to_string(), comp);

        let logic = CfmcLogic::parse("[Q1^5] AND COMP(1)").unwrap();

        // Q1 has punch 5, COMP = 1 → true AND true
        let response_line = "51";
        assert!(logic.evaluate(&questions, response_line).unwrap());

        // Q1 has punch 3, COMP = 1 → false AND true
        let response_line = "31";
        assert!(!logic.evaluate(&questions, response_line).unwrap());
    }
}
