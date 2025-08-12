use crate::rfl::RflQuestion;
use std::collections::HashMap;

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
    Plus, // +

    // Value construction
    Comma, // ,
    Dash,  // -
}

#[derive(Debug, Clone)]
pub enum CfmcNode {
    // Leaf nodes
    QuestionLabel(String),
    Literal(String),

    // Internal nodes
    Binary {
        operator: CfmcOperator,
        left: Box<CfmcNode>,
        right: Box<CfmcNode>,
    },
    Unary {
        operator: CfmcOperator,
        operand: Box<CfmcNode>,
    },
}

#[derive(Debug)]
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
        let trimmed = Self::trim_expression(expression);
        let root = Self::parse_node(&trimmed)?;
        Ok(Self { root })
    }

    fn trim_expression(expr: &str) -> String {
        let mut trimmed = expr.trim().to_uppercase();

        // Remove outer parentheses if they wrap the entire expression
        if trimmed.starts_with('(') && trimmed.ends_with(')') {
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
            }
        }

        // Remove outer brackets if they wrap the entire expression
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
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
            }
        }

        // Remove trailing $ if present
        if trimmed.ends_with('$') {
            trimmed.pop();
        }

        trimmed
    }

    fn parse_node(logic: &str) -> Result<CfmcNode, String> {
        let trimmed = Self::trim_expression(logic);

        if trimmed.is_empty() {
            return Err("Empty expression".to_string());
        }

        // Find the main operator by scanning with precedence
        if let Some((op, pos)) = Self::find_main_operator(&trimmed)? {
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
                    let operand_text = &trimmed[0..pos];
                    let operand = Self::parse_node(operand_text)?;
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
            // Check for implicit equality like COMP(1) -> COMP = (1)
            if let Some(paren_pos) = trimmed.find('(') {
                if paren_pos > 0 && !trimmed.chars().take(paren_pos).any(char::is_whitespace) {
                    let left_text = &trimmed[..paren_pos];
                    let right_text = &trimmed[paren_pos..];

                    let left = Self::parse_node(left_text)?;
                    let right = Self::parse_node(right_text)?;

                    return Ok(CfmcNode::Binary {
                        operator: CfmcOperator::Equal,
                        left: Box::new(left),
                        right: Box::new(right),
                    });
                }
            }

            // No operator found - this is a leaf node
            Ok(Self::parse_leaf(&trimmed))
        }
    }

    fn find_main_operator(logic: &str) -> Result<Option<(CfmcOperator, usize)>, String> {
        let mut level = 0;
        let mut bracket_level = 0;
        let mut in_quotes = false;
        let mut lowest_precedence = 255;
        let mut best_operator = None;
        let mut best_position = 0;

        let chars: Vec<char> = logic.chars().collect();
        let mut i = 0;

        while i < chars.len() {
            match chars[i] {
                '"' => in_quotes = !in_quotes,
                '(' if !in_quotes => {
                    level += 1;
                }
                ')' if !in_quotes => {
                    level -= 1;
                    if level < 0 {
                        return Err(format!("Unbalanced parentheses. Too many closing ')' at position {i}"));
                    }
                }
                '[' if !in_quotes => {
                    bracket_level += 1;
                }
                ']' if !in_quotes => {
                    bracket_level -= 1;
                    if bracket_level < 0 {
                        return Err(format!("Unbalanced brackets. Too many closing ']' at position {i}"));
                    }
                }
                _ => {}
            }

            if in_quotes || level > 0 || bracket_level > 0 {
                i += 1;
                continue;
            }

            // Check for operators at current position
            if let Some((op, len)) = Self::match_operator_at(&chars, i) {
                let precedence = Self::operator_precedence(&op);
                if precedence <= lowest_precedence {
                    lowest_precedence = precedence;
                    best_operator = Some(op);
                    best_position = i;
                }
                i += len;
            } else {
                i += 1;
            }
        }

        // Final check for unclosed parentheses/brackets
        if level > 0 {
            return Err(format!("Unbalanced parentheses. {level} unclosed '(' found"));
        }
        if bracket_level > 0 {
            return Err(format!("Unbalanced brackets. {bracket_level} unclosed '[' found"));
        }
        if in_quotes {
            return Err("Unclosed quote found".to_string());
        }

        Ok(best_operator.map(|op| (op, best_position)))
    }

    fn match_operator_at(chars: &[char], pos: usize) -> Option<(CfmcOperator, usize)> {
        if pos >= chars.len() {
            return None;
        }

        let remaining: String = chars[pos..].iter().collect();

        // Check longer operators first
        if remaining.starts_with("NUMITEMS") {
            Some((CfmcOperator::NumItems, 8))
        } else if remaining.starts_with("NOT") {
            Some((CfmcOperator::Not, 3))
        } else if remaining.starts_with("AND") {
            Some((CfmcOperator::And, 3))
        } else if remaining.starts_with("OR") {
            Some((CfmcOperator::Or, 2))
        } else if remaining.starts_with("<=") {
            Some((CfmcOperator::LessEqual, 2))
        } else if remaining.starts_with(">=") {
            Some((CfmcOperator::GreaterEqual, 2))
        } else if remaining.starts_with("<>") {
            Some((CfmcOperator::NotEqual, 2))
        } else if remaining.starts_with("^^NB") {
            Some((CfmcOperator::IsNotBlank, 4))
        } else if remaining.starts_with("^^B") {
            Some((CfmcOperator::IsBlank, 3))
        } else if remaining.starts_with('<') {
            Some((CfmcOperator::Less, 1))
        } else if remaining.starts_with('>') {
            Some((CfmcOperator::Greater, 1))
        } else if remaining.starts_with('=')
            || remaining.starts_with('$')
            || remaining.starts_with('#')
        {
            Some((CfmcOperator::Equal, 1))
        } else if remaining.starts_with('+') {
            Some((CfmcOperator::Plus, 1))
        } else if remaining.starts_with(',') {
            Some((CfmcOperator::Comma, 1))
        } else if remaining.starts_with('-') {
            Some((CfmcOperator::Dash, 1))
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
            CfmcOperator::Comma | CfmcOperator::Dash => 50,
            CfmcOperator::Not
            | CfmcOperator::NumItems
            | CfmcOperator::IsBlank
            | CfmcOperator::IsNotBlank => 60,
        }
    }

    const fn operator_length(op: &CfmcOperator) -> usize {
        match op {
            CfmcOperator::NumItems => 8,
            CfmcOperator::IsNotBlank => 4,
            CfmcOperator::Not | CfmcOperator::And | CfmcOperator::IsBlank => 3,
            CfmcOperator::Or
            | CfmcOperator::LessEqual
            | CfmcOperator::GreaterEqual
            | CfmcOperator::NotEqual => 2,
            _ => 1,
        }
    }

    fn parse_leaf(text: &str) -> CfmcNode {
        let trimmed = text.trim();

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

    /// Evaluate the logic expression against survey responses
    ///
    /// # Errors
    ///
    /// Returns an error if a question reference is not found in the RFL or if evaluation fails
    pub fn evaluate(
        &self,
        questions: &HashMap<String, RflQuestion>,
        response_line: &str,
    ) -> Result<bool, String> {
        self.evaluate_node(&self.root, questions, response_line)
    }

    fn evaluate_node(
        &self,
        node: &CfmcNode,
        questions: &HashMap<String, RflQuestion>,
        response_line: &str,
    ) -> Result<bool, String> {
        match node {
            CfmcNode::QuestionLabel(label) => {
                questions.get(label).map_or_else(
                    || Err(format!("Question {label} not found in RFL")),
                    |question| {
                        let responses = question.responses(response_line);
                        // Question reference evaluates to true if any response is non-empty
                        Ok(responses.iter().any(|r| !r.trim().is_empty()))
                    },
                )
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
                    let left_val = self.evaluate_node(left, questions, response_line)?;
                    let right_val = self.evaluate_node(right, questions, response_line)?;
                    Ok(left_val && right_val)
                }

                CfmcOperator::Or => {
                    let left_val = self.evaluate_node(left, questions, response_line)?;
                    let right_val = self.evaluate_node(right, questions, response_line)?;
                    Ok(left_val || right_val)
                }

                CfmcOperator::Equal => {
                    self.evaluate_equality(left, right, questions, response_line)
                }

                CfmcOperator::Plus => {
                    // Plus operator returns substring - this shouldn't be evaluated as boolean directly
                    Err("Plus operator should not be used as a standalone boolean expression".to_string())
                }

                CfmcOperator::Less
                | CfmcOperator::LessEqual
                | CfmcOperator::Greater
                | CfmcOperator::GreaterEqual => {
                    self.evaluate_inequality(left, right, questions, response_line, operator)
                }

                _ => Err(format!("Binary operator {operator:?} not yet implemented")),
            },

            CfmcNode::Unary { operator, operand } => match operator {
                CfmcOperator::Not => {
                    let val = self.evaluate_node(operand, questions, response_line)?;
                    Ok(!val)
                }

                CfmcOperator::IsBlank => {
                    Self::evaluate_blank(operand, questions, response_line)
                }

                CfmcOperator::IsNotBlank => {
                    Ok(!Self::evaluate_blank(operand, questions, response_line)?)
                }

                _ => Err(format!("Unary operator {operator:?} not yet implemented")),
            },
        }
    }

    fn evaluate_equality(
        &self,
        left: &CfmcNode,
        right: &CfmcNode,
        questions: &HashMap<String, RflQuestion>,
        response_line: &str,
    ) -> Result<bool, String> {
        // Get responses from left side
        let left_responses = match left {
            CfmcNode::QuestionLabel(label) => {
                if let Some(question) = questions.get(label) {
                    question.responses(response_line)
                } else {
                    return Err(format!("Question {label} not found in RFL"));
                }
            }
            CfmcNode::Binary { 
                operator: CfmcOperator::Plus, 
                left: plus_left, 
                right: plus_right 
            } => {
                // Handle Plus operation on left side - extract substring
                Self::evaluate_substring(plus_left, plus_right, questions, response_line)?
            }
            _ => return Err("Left side of equality must be a question label or plus expression".to_string()),
        };

        // Get expected value(s) from right side
        let expected_values = self.get_value_list(right, questions, response_line)?;

        // Check if any response matches any expected value
        Ok(Self::responses_intersect(&left_responses, &expected_values))
    }

    fn evaluate_inequality(
        &self,
        left: &CfmcNode,
        right: &CfmcNode,
        questions: &HashMap<String, RflQuestion>,
        response_line: &str,
        operator: &CfmcOperator,
    ) -> Result<bool, String> {
        // Get responses from left side (should be a question)
        let left_responses = match left {
            CfmcNode::QuestionLabel(label) => {
                if let Some(question) = questions.get(label) {
                    question.responses(response_line)
                } else {
                    return Err(format!("Question {label} not found in RFL"));
                }
            }
            _ => return Err("Left side of equality must be a question label".to_string()),
        };

        // Get expected value(s) from right side
        let expected_values = self.get_value_list(right, questions, response_line)?;

        // Check if any response matches any expected value
        Ok(Self::responses_comparison(
            &left_responses,
            &expected_values,
            operator,
        ))
    }

    fn evaluate_blank(
        left: &CfmcNode,
        questions: &HashMap<String, RflQuestion>,
        response_line: &str,
    ) -> Result<bool, String> {
        // Get responses from left side (should be a question)
        let left_responses = match left {
            CfmcNode::QuestionLabel(label) => {
                if let Some(question) = questions.get(label) {
                    question.responses(response_line)
                } else {
                    return Err(format!("Question {label} not found in RFL"));
                }
            }
            _ => return Err("Left side of equality must be a question label".to_string()),
        };

        // Check if any response matches any expected value
        Ok(left_responses.join("").trim().is_empty())
    }

    fn evaluate_substring(
        left: &CfmcNode,
        right: &CfmcNode,
        questions: &HashMap<String, RflQuestion>,
        response_line: &str,
    ) -> Result<Vec<String>, String> {
        // Get responses from left side (should be a question)
        let left_responses = match left {
            CfmcNode::QuestionLabel(label) => {
                if let Some(question) = questions.get(label) {
                    question.responses(response_line)
                } else {
                    return Err(format!("Question {label} not found in RFL"));
                }
            }
            _ => return Err("Left side of plus sign must be a question label".to_string()),
        };

        // Get column.width specification from right side - should be a literal like "0.1"
        let CfmcNode::Literal(spec) = right else {
            return Err("Right side of plus operator must be a column.width specification".to_string());
        };

        // Parse column.width specification
        if let Some((col_str, width_str)) = spec.split_once('.') {
            let col = col_str.parse::<usize>().map_err(|_| format!("Invalid column number: {col_str}"))?;
            let width = width_str.parse::<usize>().map_err(|_| format!("Invalid width: {width_str}"))?;
            
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
        &self,
        node: &CfmcNode,
        questions: &HashMap<String, RflQuestion>,
        response_line: &str,
    ) -> Result<Vec<String>, String> {
        match node {
            CfmcNode::Literal(value) => Ok(vec![value.clone()]),
            CfmcNode::QuestionLabel(label) => questions.get(label).map_or_else(
                || Err(format!("Question {label} not found in RFL")),
                |question| Ok(question.responses(response_line)),
            ),
            CfmcNode::Binary {
                operator: CfmcOperator::Comma,
                left,
                right,
            } => {
                let mut left_values = self.get_value_list(left, questions, response_line)?;
                let mut right_values = self.get_value_list(right, questions, response_line)?;
                left_values.append(&mut right_values);
                Ok(left_values)
            }
            CfmcNode::Binary {
                operator: CfmcOperator::Dash,
                left,
                right,
            } => {
                let left_values = self.get_value_list(left, questions, response_line)?;
                let right_values = self.get_value_list(right, questions, response_line)?;
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
                        if response_trimmed.len() == start.len()
                            && response_trimmed.as_str() >= start
                            && response_trimmed.as_str() <= end
                        {
                            return true;
                        }
                    }
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
                    CfmcOperator::Comma => ",",
                    CfmcOperator::Dash => "-",
                    _ => &format!(" {operator:?} "),
                };

                let result = match operator {
                    CfmcOperator::Equal => format!("{left_str}({right_str})"),
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
                // Don't pass unary operators as parent context - they don't affect precedence
                let operand_str = Self::node_to_string_with_context(operand, parent_op);
                let op_str = match operator {
                    CfmcOperator::Not => "NOT ",
                    CfmcOperator::NumItems => "NUMITEMS ",
                    _ => &format!("{operator:?} "),
                };
                format!("{op_str}{operand_str}")
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
        assert_eq!(output, "NOT COMP(3)");
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
    fn test_evaluate_simple_equality() {
        let mut questions = HashMap::new();
        let question = RflQuestion {
            label: "COMP".to_string(),
            start_col: 1,
            width: 1,
            question_type: QuestionType::Fld,
            max_responses: 1,
            text_lines: Vec::new(),
            response_codes: HashMap::new(),
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
        let mut questions = HashMap::new();

        let comp_question = RflQuestion {
            label: "COMP".to_string(),
            start_col: 1,
            width: 1,
            question_type: QuestionType::Fld,
            max_responses: 1,
            text_lines: Vec::new(),
            response_codes: HashMap::new(),
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
            response_codes: HashMap::new(),
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
        let mut questions = HashMap::new();
        let question = RflQuestion {
            label: "COMP".to_string(),
            start_col: 1,
            width: 1,
            question_type: QuestionType::Fld,
            max_responses: 1,
            text_lines: Vec::new(),
            response_codes: HashMap::new(),
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
        let mut questions = HashMap::new();
        let question = RflQuestion {
            label: "QB".to_string(),
            start_col: 1,
            width: 2,
            question_type: QuestionType::Fld,
            max_responses: 1,
            text_lines: Vec::new(),
            response_codes: HashMap::new(),
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
    fn test_evaluate_plus_expression() {
        let mut questions = HashMap::new();
        let q02_question = RflQuestion {
            label: "Q02".to_string(),
            start_col: 1,
            width: 3,  // Make room for 3 characters
            question_type: QuestionType::Fld,
            max_responses: 1,
            text_lines: Vec::new(),
            response_codes: HashMap::new(),
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
}
