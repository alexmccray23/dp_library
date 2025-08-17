use regex::Regex;
use std::{
    collections::HashMap,
    fs::File,
    io::{BufRead, BufReader, Result as IoResult},
    sync::LazyLock,
};

static QUESTION_PREFIX_RE: LazyLock<Regex> =
    LazyLock::new(|| regex::Regex::new(r"^[A-Za-z]*\d+[A-Za-z]*\.?\s*").unwrap());

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuestionType {
    Fld, // Field (categorical)
    Var, // Variable
    Num, // Numeric
    Exp, // Expression
}

impl QuestionType {
    fn from_str(s: &str) -> Option<Self> {
        match s.to_uppercase().as_str() {
            "FLD" => Some(Self::Fld),
            "VAR" => Some(Self::Var),
            "NUM" => Some(Self::Num),
            "EXP" => Some(Self::Exp),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RflQuestion {
    pub label: String,
    pub text_lines: Vec<String>,
    pub response_codes: HashMap<String, String>,
    pub exceptions: Vec<String>,
    pub start_col: usize,
    pub width: usize,
    pub max_responses: usize,
    pub min_value: Option<i32>,
    pub max_value: Option<i32>,
    pub question_type: QuestionType,
}

impl RflQuestion {
    #[must_use]
    pub fn new_case_id(start_col: usize, width: usize) -> Self {
        Self {
            label: "CASEID".to_string(),
            start_col,
            width,
            question_type: QuestionType::Num,
            max_responses: 1,
            text_lines: vec!["CASE ID".to_string()],
            response_codes: HashMap::new(),
            min_value: None,
            max_value: None,
            exceptions: Vec::new(),
        }
    }

    /// Parse an RFL question from multiple lines
    ///
    /// # Errors
    ///
    /// Returns an error if the question format is invalid
    pub fn from_lines(lines: &[String]) -> Result<Self, String> {
        let mut question = Self {
            label: String::new(),
            start_col: 0,
            width: 1,
            question_type: QuestionType::Fld,
            max_responses: 1,
            text_lines: Vec::new(),
            response_codes: HashMap::new(),
            min_value: None,
            max_value: None,
            exceptions: Vec::new(),
        };

        for line in lines {
            if line.starts_with('Q') && line.chars().nth(1) == Some(' ') {
                question.parse_question_line(line)?;
            } else if line.starts_with('T') && line.chars().nth(1) == Some(' ') {
                question.text_lines.push(line[2..].to_string());
            } else if line.starts_with('R') && line.chars().nth(1) == Some(' ') {
                question.parse_response_line(line);
            } else if line.starts_with('X') && question.question_type == QuestionType::Num {
                question.parse_numeric_range_line(line);
            }
        }

        Ok(question)
    }

    fn parse_question_line(&mut self, line: &str) -> Result<(), String> {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 6 {
            return Err(format!("Invalid question line format: {line}"));
        }

        self.label = parts[1].to_string().to_uppercase();
        self.question_type = QuestionType::from_str(parts[2])
            .ok_or_else(|| format!("Invalid question type: {}", parts[2]))?;

        // Parse location [startcol.width]
        let location_str = parts[5].trim_matches(['[', ']']);
        let location_parts: Vec<&str> = location_str.split('.').collect();

        self.start_col = location_parts[0]
            .parse()
            .map_err(|_| format!("Invalid start column: {}", location_parts[0]))?;

        if location_parts.len() > 1 {
            let total_width: usize = location_parts[1]
                .parse()
                .map_err(|_| format!("Invalid width: {}", location_parts[1]))?;

            // Look for MAX= parameter
            if let Some(max_part) = parts.iter().find(|p| p.starts_with("Max=")) {
                let max_str = max_part.strip_prefix("Max=").unwrap();
                self.max_responses = max_str
                    .parse()
                    .map_err(|_| format!("Invalid max responses: {max_str}"))?;
                self.width = total_width / self.max_responses;
            } else {
                self.width = total_width;
            }
        }

        Ok(())
    }

    fn parse_response_line(&mut self, line: &str) {
        // R     1                                                             COMPLETE
        // Skip the "R " and then split on whitespace, but preserve everything after the code
        if let Some(after_r) = line.strip_prefix("R ") {
            let trimmed = after_r.trim_start();
            if let Some(space_pos) = trimmed.find(' ') {
                let code = trimmed[..space_pos].trim().to_string();
                let text = trimmed[space_pos..].trim().to_string();
                self.response_codes.insert(code, text);
            }
        }
    }

    fn parse_numeric_range_line(&mut self, line: &str) {
        // X Range=1913-2005            Exceptions=9999
        if let Some(range_start) = line.find("Range=") {
            let range_part = &line[range_start + 6..];
            if let Some(dash_pos) = range_part.find('-') {
                let min_str = range_part[..dash_pos].trim();
                let rest = &range_part[dash_pos + 1..];
                let max_str = rest
                    .find(' ')
                    .map_or_else(|| rest.trim(), |space_pos| rest[..space_pos].trim());

                self.min_value = min_str.parse().ok();
                self.max_value = max_str.parse().ok();
            }
        }

        if let Some(exc_start) = line.find("Exceptions=") {
            let exc_part = &line[exc_start + 11..].trim();
            if !exc_part.is_empty() {
                self.exceptions = exc_part
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
        }
    }

    #[must_use]
    pub fn main_text(&self) -> Vec<String> {
        self.text_lines
            .iter()
            .filter_map(|line| {
                let trimmed = line.trim();
                if trimmed.starts_with("DATAFROM(0.1)")
                    || trimmed.is_empty()
                    || trimmed.contains("ENTER NUMBER")
                    || (trimmed.starts_with('(') && trimmed.ends_with(')'))
                {
                    None
                } else {
                    // Remove question numbering prefixes like "A1.", "Q17.", etc.
                    let cleaned = Self::remove_question_prefix(trimmed);
                    Some(cleaned)
                }
            })
            .collect()
    }

    fn remove_question_prefix(text: &str) -> String {
        // Remove patterns like "A1.", "Q17.", "D16C.", etc.
        QUESTION_PREFIX_RE.replace(text, "").to_string()
    }

    #[must_use]
    pub fn responses(&self, response_line: &str) -> Vec<String> {
        let mut responses = Vec::new();
        for i in 0..self.max_responses {
            let start_pos = (self.start_col - 1) + (i * self.width);
            if start_pos + self.width <= response_line.len() {
                let response = response_line[start_pos..start_pos + self.width].to_string();
                responses.push(response);
            }
        }
        responses
    }
}

#[derive(Debug)]
pub struct RflFile {
    questions: HashMap<String, RflQuestion>,
    questions_array: Vec<RflQuestion>,
}

impl RflFile {
    /// Load RFL file from filesystem
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read
    pub fn from_file(filename: &str) -> IoResult<Self> {
        let file = File::open(filename)?;
        let reader = BufReader::new(file);
        let lines: Result<Vec<String>, _> = reader.lines().collect();
        let lines = lines?;

        Self::from_lines(&lines)
    }

    /// Parse RFL data from lines of text
    ///
    /// # Errors
    ///
    /// Returns an error if parsing fails
    pub fn from_lines(lines: &[String]) -> IoResult<Self> {
        let mut rfl = Self {
            questions: HashMap::new(),
            questions_array: Vec::new(),
        };

        let mut i = 0;
        while i < lines.len() {
            let line = &lines[i];

            // Check for case ID line
            if line.starts_with("The case ID will be in columns ") {
                if let Some(location_str) = line.strip_prefix("The case ID will be in columns ") {
                    let parts: Vec<&str> = location_str.split('.').collect();
                    if parts.len() == 2
                        && let (Ok(start), Ok(width)) = (parts[0].parse(), parts[1].parse()) {
                            let question = RflQuestion::new_case_id(start, width);
                            rfl.questions
                                .insert(question.label.clone(), question.clone());
                            rfl.questions_array.push(question);
                        }
                }
                i += 1;
                continue;
            }

            // Check for question line
            if line.starts_with('Q') && line.len() > 1 && line.chars().nth(1) == Some(' ') {
                let mut question_lines = vec![line.clone()];
                i += 1;

                // Collect all related lines (T, R, X)
                while i < lines.len() {
                    let next_line = &lines[i];
                    if next_line.starts_with('Q')
                        && next_line.len() > 1
                        && next_line.chars().nth(1) == Some(' ')
                    {
                        break; // Start of next question
                    }
                    if (next_line.starts_with('T')
                        || next_line.starts_with('R')
                        || next_line.starts_with('X'))
                        && next_line.len() > 1
                        && next_line.chars().nth(1) == Some(' ')
                    {
                        question_lines.push(next_line.clone());
                    }
                    i += 1;
                }

                match RflQuestion::from_lines(&question_lines) {
                    Ok(question) => {
                        rfl.questions
                            .insert(question.label.clone(), question.clone());
                        rfl.questions_array.push(question);
                    }
                    Err(e) => {
                        eprintln!("Warning: Failed to parse question: {e}");
                    }
                }
            } else {
                i += 1;
            }
        }

        Ok(rfl)
    }

    #[must_use]
    pub const fn questions(&self) -> &HashMap<String, RflQuestion> {
        &self.questions
    }

    #[must_use]
    pub fn questions_array(&self) -> &[RflQuestion] {
        &self.questions_array
    }

    #[must_use]
    pub fn get_question(&self, label: &str) -> Option<&RflQuestion> {
        self.questions.get(label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_case_id() {
        let lines = vec!["The case ID will be in columns 1.9".to_string()];
        let rfl = RflFile::from_lines(&lines).unwrap();

        let caseid = rfl.get_question("CASEID").unwrap();
        assert_eq!(caseid.start_col, 1);
        assert_eq!(caseid.width, 9);
        assert_eq!(caseid.question_type, QuestionType::Num);
    }

    #[test]
    fn test_parse_field_question() {
        let lines = vec![
            "Q COMP                           FLD  [2942]         --> [10]".to_string(),
            "T COMP".to_string(),
            "R     1                                                             COMPLETE".to_string(),
            "R     2                                                             QUALIFIED TERMINATE".to_string(),
        ];

        let rfl = RflFile::from_lines(&lines).unwrap();
        let question = rfl.get_question("COMP").unwrap();

        assert_eq!(question.label, "COMP");
        assert_eq!(question.start_col, 10);
        assert_eq!(question.width, 1);
        assert_eq!(question.question_type, QuestionType::Fld);
        assert_eq!(question.text_lines.len(), 1);
        assert_eq!(question.text_lines[0], "COMP");
        assert_eq!(question.response_codes.len(), 2);
        assert_eq!(question.response_codes.get("1").unwrap(), "COMPLETE");
    }

    #[test]
    fn test_parse_numeric_question() {
        let lines = vec![
            "Q QD1                            NUM  [2973.4]       --> [34.4]".to_string(),
            "X Range=1913-2005            Exceptions=9999".to_string(),
            "T Now, I am going to ask you a few questions about yourself".to_string(),
        ];

        let rfl = RflFile::from_lines(&lines).unwrap();
        let question = rfl.get_question("QD1").unwrap();

        assert_eq!(question.label, "QD1");
        assert_eq!(question.start_col, 34);
        assert_eq!(question.width, 4);
        assert_eq!(question.question_type, QuestionType::Num);
        assert_eq!(question.min_value, Some(1913));
        assert_eq!(question.max_value, Some(2005));
        assert_eq!(question.exceptions, vec!["9999".to_string()]);
    }

    #[test]
    fn test_question_with_max_responses() {
        let lines = vec![
            "Q QB                             FLD  [2960.2]       --> [25.2]".to_string(),
            "T In which state do you currently live?".to_string(),
        ];

        let rfl = RflFile::from_lines(&lines).unwrap();
        let question = rfl.get_question("QB").unwrap();

        assert_eq!(question.start_col, 25);
        assert_eq!(question.width, 2);
        assert_eq!(question.max_responses, 1);
    }

    #[test]
    fn test_responses_extraction() {
        let question = RflQuestion {
            label: "TEST".to_string(),
            start_col: 5,
            width: 2,
            question_type: QuestionType::Fld,
            max_responses: 3,
            text_lines: Vec::new(),
            response_codes: HashMap::new(),
            min_value: None,
            max_value: None,
            exceptions: Vec::new(),
        };

        let response_line = "123456789012345678901234567890";
        let responses = question.responses(response_line);

        assert_eq!(responses.len(), 3);
        assert_eq!(responses[0], "56"); // positions 5-6 (0-indexed: 4-5)
        assert_eq!(responses[1], "78"); // positions 7-8 (0-indexed: 6-7)
        assert_eq!(responses[2], "90"); // positions 9-10 (0-indexed: 8-9)
    }

    #[test]
    fn test_main_text_filtering() {
        let question = RflQuestion {
            label: "TEST".to_string(),
            start_col: 1,
            width: 1,
            question_type: QuestionType::Fld,
            max_responses: 1,
            text_lines: vec![
                " ".to_string(),
                "DATAFROM(0.1) should be filtered".to_string(),
                "A1. This is a real question".to_string(),
                String::new(),
                "ENTER NUMBER should be filtered".to_string(),
                "Q17. Another question".to_string(),
            ],
            response_codes: HashMap::new(),
            min_value: None,
            max_value: None,
            exceptions: Vec::new(),
        };

        let main_text = question.main_text();
        assert_eq!(main_text.len(), 2);
        assert_eq!(main_text[0], "This is a real question");
        assert_eq!(main_text[1], "Another question");
    }

    #[test]
    fn test_parse_real_rfl_file() {
        let rfl = RflFile::from_file("p0116.rfl");
        match rfl {
            Ok(rfl_file) => {
                let questions = rfl_file.questions();
                println!("Successfully parsed {} questions", questions.len());

                // Test case ID
                if let Some(caseid) = rfl_file.get_question("CASEID") {
                    assert_eq!(caseid.start_col, 1);
                    assert_eq!(caseid.width, 9);
                }

                // Test a field question
                if let Some(comp) = rfl_file.get_question("COMP") {
                    assert_eq!(comp.question_type, QuestionType::Fld);
                    assert!(comp.response_codes.contains_key("1"));
                }

                // Test a numeric question
                if let Some(qd1) = rfl_file.get_question("QD1") {
                    assert_eq!(qd1.question_type, QuestionType::Num);
                    assert_eq!(qd1.min_value, Some(1913));
                    assert_eq!(qd1.max_value, Some(2005));
                }
            }
            Err(_) => {
                // File doesn't exist in test environment, skip test
                println!("RFL file not found, skipping integration test");
            }
        }
    }
}
