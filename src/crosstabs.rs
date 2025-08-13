use crate::rfl::RflQuestion;
use calamine::{Data, Reader, open_workbook_auto};
use regex::Regex;
use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::fmt::Write;

#[derive(Debug, Clone)]
pub enum CrossTabsError {
    ParseError(String),
    ExcelError(String),
    QuestionNotFound(String),
}

impl fmt::Display for CrossTabsError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::ParseError(msg) => write!(f, "Parse error: {msg}"),
            Self::ExcelError(msg) => write!(f, "Excel error: {msg}"),
            Self::QuestionNotFound(msg) => write!(f, "Question not found: {msg}"),
        }
    }
}

impl Error for CrossTabsError {}

#[derive(Debug, Clone)]
pub enum CrossTabsNode {
    Binary {
        left: Box<CrossTabsNode>,
        operator: String,
        right: Box<CrossTabsNode>,
    },
    Unary {
        operator: String,
        operand: Box<CrossTabsNode>,
    },
    Comparison {
        question: String,
        codes: String,
    },
    Literal(String),
    Group(Box<CrossTabsNode>),
}

#[derive(Debug, Clone)]
pub struct CrossTabsLogic {
    pub root: CrossTabsNode,
}

impl CrossTabsLogic {
    /// Create a new `CrossTabsLogic` instance from a logic expression.
    ///
    /// # Errors
    ///
    /// Returns `CrossTabsError::ParseError` if the logic expression cannot be parsed.
    pub fn new(logic: &str) -> Result<Self, CrossTabsError> {
        let processed = Self::preprocess_logic(logic);
        let root = Self::parse_expression(&processed)?;
        Ok(Self { root })
    }

    fn preprocess_logic(logic: &str) -> String {
        let mut result = logic.to_string();

        // Handle slash syntax: QX2/Q17:1 becomes QX2:1 OR Q17:1
        let slash_re = Regex::new(r"(\w+)/(\w+)(:\d+(?:-\d+|,\d+)*)").unwrap();
        result = slash_re.replace_all(&result, "$1$3 OR $2$3").to_string();

        // Handle D prefix -> Q prefix
        let d_re = Regex::new(r"(\(*)(D\d)").unwrap();
        result = d_re.replace_all(&result, "${1}Q$2").to_string();

        result
    }

    fn parse_expression(input: &str) -> Result<CrossTabsNode, CrossTabsError> {
        let trimmed = input.trim();

        if trimmed.is_empty() {
            return Err(CrossTabsError::ParseError("Empty expression".to_string()));
        }

        // Handle MINUS operator (highest precedence)
        if let Some(pos) = Self::find_operator(trimmed, " MINUS ") {
            let left = Self::parse_expression(&trimmed[..pos])?;
            let right = Self::parse_expression(&trimmed[pos + 7..])?;
            return Ok(CrossTabsNode::Binary {
                left: Box::new(left),
                operator: "--".to_string(),
                right: Box::new(right),
            });
        }

        // Handle AND operator
        if let Some(pos) = Self::find_operator(trimmed, " AND ") {
            let left = Self::parse_expression(&trimmed[..pos])?;
            let right = Self::parse_expression(&trimmed[pos + 5..])?;
            return Ok(CrossTabsNode::Binary {
                left: Box::new(left),
                operator: "&".to_string(),
                right: Box::new(right),
            });
        }

        // Handle & operator
        if let Some(pos) = Self::find_operator(trimmed, " & ") {
            let left = Self::parse_expression(&trimmed[..pos])?;
            let right = Self::parse_expression(&trimmed[pos + 3..])?;
            return Ok(CrossTabsNode::Binary {
                left: Box::new(left),
                operator: "&".to_string(),
                right: Box::new(right),
            });
        }

        // Handle OR operator (lowest precedence)
        if let Some(pos) = Self::find_operator(trimmed, " OR ") {
            let left = Self::parse_expression(&trimmed[..pos])?;
            let right = Self::parse_expression(&trimmed[pos + 4..])?;
            return Ok(CrossTabsNode::Binary {
                left: Box::new(left),
                operator: "|".to_string(),
                right: Box::new(right),
            });
        }

        // Handle parentheses
        if trimmed.starts_with('(') && trimmed.ends_with(')') {
            let inner = &trimmed[1..trimmed.len() - 1];
            let inner_node = Self::parse_expression(inner)?;
            return Ok(CrossTabsNode::Group(Box::new(inner_node)));
        }

        // Handle question:code patterns
        if let Some(colon_pos) = trimmed.find(':') {
            let question = trimmed[..colon_pos].trim().to_string();
            let codes = trimmed[colon_pos + 1..]
                .trim()
                .to_string()
                .replace(", ", "");
            return Ok(CrossTabsNode::Comparison { question, codes });
        }

        // Default to literal
        Ok(CrossTabsNode::Literal(trimmed.to_string()))
    }

    fn find_operator(input: &str, op: &str) -> Option<usize> {
        let mut paren_depth = 0;
        let chars: Vec<char> = input.chars().collect();
        let op_chars: Vec<char> = op.chars().collect();

        for i in 0..=chars.len().saturating_sub(op_chars.len()) {
            match chars[i] {
                '(' => paren_depth += 1,
                ')' => paren_depth -= 1,
                _ => {}
            }

            if paren_depth == 0 && chars[i..].starts_with(&op_chars) {
                return Some(i);
            }
        }
        None
    }

    #[must_use]
    pub fn to_inorder(&self) -> String {
        Self::node_to_inorder(&self.root)
    }

    fn node_to_inorder(node: &CrossTabsNode) -> String {
        match node {
            CrossTabsNode::Binary {
                left,
                operator,
                right,
            } => {
                let left_str = Self::node_to_inorder(left);
                let right_str = Self::node_to_inorder(right);
                let op_str = match operator.as_str() {
                    "--" => " MINUS ",
                    "&" => " & ",
                    "|" => " OR ",
                    _ => &format!(" {operator} "),
                };
                format!("{left_str}{op_str}{right_str}")
            }
            CrossTabsNode::Unary { operator, operand } => {
                format!("{}{}", operator, Self::node_to_inorder(operand))
            }
            CrossTabsNode::Comparison { question, codes } => {
                format!("{question}:{codes}")
            }
            CrossTabsNode::Literal(value) => value.clone(),
            CrossTabsNode::Group(inner) => {
                format!("({})", Self::node_to_inorder(inner))
            }
        }
    }

    /// Convert the logic expression to Uncle syntax.
    #[must_use]
    pub fn to_uncle_syntax(&self, questions: &HashMap<String, RflQuestion>) -> String {
        Self::node_to_uncle_syntax(&self.root, questions)
    }

    fn node_to_uncle_syntax(
        node: &CrossTabsNode,
        questions: &HashMap<String, RflQuestion>,
    ) -> String {
        match node {
            CrossTabsNode::Binary {
                left,
                operator,
                right,
            } => {
                let left_str = Self::node_to_uncle_syntax(left, questions);
                let right_str = Self::node_to_uncle_syntax(right, questions);
                let op_str = match operator.as_str() {
                    "--" => "-",
                    "&" => " ",
                    "|" => " OR ",
                    _ => &format!(" {operator} "),
                };
                format!("{left_str}{op_str}{right_str}")
            }
            CrossTabsNode::Unary { operator, operand } => {
                let operand_str = Self::node_to_uncle_syntax(operand, questions);
                format!("{operator}{operand_str}")
            }
            CrossTabsNode::Comparison { question, codes } => {
                Self::comparison_to_uncle_syntax(question, codes, questions)
            }
            // CrossTabsNode::Literal(value) => value.clone(),
            CrossTabsNode::Literal(_) => String::new(),
            CrossTabsNode::Group(inner) => {
                let inner_str = Self::node_to_uncle_syntax(inner, questions);
                format!("({inner_str})")
            }
        }
    }

    fn comparison_to_uncle_syntax(
        question_name: &str,
        codes: &str,
        questions: &HashMap<String, RflQuestion>,
    ) -> String {
        // Try to find the question with or without Q prefix
        let question = questions
            .get(question_name.to_uppercase().as_str())
            .or_else(|| questions.get(&format!("Q{}", question_name.to_uppercase())));

        let Some(question) = question else {
            eprintln!("Question {question_name} is in spec but not found in rfl.");
            // Return a fallback syntax when question is not found
            return format!("{question_name}:{codes}");
        };

        let mut processed_codes = codes.replace('-', ":");

        // Handle MEAN special case
        if codes == "MEAN" {
            let start_col = question.start_col;
            let end_col = start_col + question.width - 1;
            let mean_part = format!("A(1!{start_col}:{end_col}), ");
            processed_codes = format!(
                "{}:{}",
                question.min_value.unwrap_or(0),
                question.max_value.unwrap_or(99)
            );
            return format!(
                "{}{}",
                mean_part,
                Self::generate_uncle_syntax(question, &processed_codes)
            );
        }

        Self::generate_uncle_syntax(question, &processed_codes)
    }

    fn generate_uncle_syntax(question: &RflQuestion, codes: &str) -> String {
        let start_col = question.start_col;
        let width = question.width;
        let max_responses = question.max_responses;

        if width == 1 {
            if max_responses == 1 {
                format!("1!{start_col}-{codes}")
            } else {
                format!(
                    "1!{}:{}-{}",
                    start_col,
                    start_col + max_responses - 1,
                    codes
                )
            }
        } else if max_responses == 1 {
            let end_col = start_col + width - 1;
            format!("R(1!{start_col}:{end_col},{codes})")
        } else {
            let mut ranges = Vec::new();
            for i in 0..max_responses {
                let range_start = start_col + (i * width);
                let range_end = range_start + width - 1;
                ranges.push(format!("1!{range_start}:{range_end}"));
            }
            format!("R({},{})", ranges.join("/"), codes)
        }
    }
}

#[derive(Debug, Clone)]
pub struct CrossTab {
    pub title: String,
    pub specs: CrossTabsLogic,
    pub base: String,
    pub options: String,
}

impl CrossTab {
    /// Create a new `CrossTab`.
    ///
    /// # Errors
    ///
    /// Returns `CrossTabsError::ParseError` if the specs cannot be parsed.
    pub fn new(
        title: &str,
        specs: &str,
        base: &str,
        options: &str,
    ) -> Result<Self, CrossTabsError> {
        let mut processed_options = options.to_string();
        if specs.to_uppercase().contains("MEAN") {
            processed_options = format!("NOVP FDP 0 {processed_options}");
        }

        Ok(Self {
            title: title.trim().to_string(),
            specs: CrossTabsLogic::new(specs)?,
            base: base.to_string(),
            options: processed_options,
        })
    }

    #[must_use]
    pub fn uncle_title(&self) -> String {
        self.title
            .to_uppercase()
            .replace('/', "//")
            .replace('=', "==")
    }
}

#[derive(Debug, Clone)]
pub struct Banner {
    pub title: String,
    pub subtitle: String,
    pub specs: CrossTabsLogic,
    pub base: String,
    pub options: String,
}

impl Banner {
    /// Create a new Banner.
    ///
    /// # Errors
    ///
    /// Returns `CrossTabsError::ParseError` if the specs cannot be parsed.
    pub fn new(
        title: &str,
        subtitle: &str,
        specs: &str,
        base: &str,
        options: &str,
    ) -> Result<Self, CrossTabsError> {
        Ok(Self {
            title: title.trim().to_string(),
            subtitle: subtitle.trim().to_string(),
            specs: CrossTabsLogic::new(specs)?,
            base: base.to_string(),
            options: options.to_string(),
        })
    }

    #[must_use]
    pub fn uncle_title(&self) -> String {
        self.title
            .to_uppercase()
            .replace('/', "//")
            .replace('=', "==")
    }

    #[must_use]
    pub fn uncle_subtitle(&self) -> String {
        self.subtitle
            .to_uppercase()
            .replace('/', "//")
            .replace('&', "&&")
            .replace('=', "==")
            .trim_end()
            .to_string()
    }
}

#[derive(Debug)]
pub struct CrossTabsTable {
    pub index: String,
    pub title: String,
    pub crosstabs: Vec<CrossTab>,
    pub include_text: bool,
}

impl CrossTabsTable {
    #[must_use]
    pub fn new(index: &str, title: &str, include_text: bool) -> Self {
        let cleaned_index = index.chars().filter(char::is_ascii_digit).collect();

        Self {
            index: cleaned_index,
            title: title.to_string(),
            crosstabs: Vec::new(),
            include_text,
        }
    }

    /// Add a crosstab to the table.
    ///
    /// # Errors
    ///
    /// Returns `CrossTabsError::ParseError` if the specs cannot be parsed.
    pub fn add_crosstab(
        &mut self,
        title: &str,
        specs: &str,
        base: &str,
        options: &str,
    ) -> Result<(), CrossTabsError> {
        let crosstab = CrossTab::new(title, specs, base, options)?;
        self.crosstabs.push(crosstab);
        Ok(())
    }

    pub fn check_bases(&mut self) {
        let mut additional_crosstabs = Vec::new();

        for crosstab in &self.crosstabs {
            if crosstab.base != "ALL" {
                let base_exists = self
                    .crosstabs
                    .iter()
                    .any(|ct| ct.specs.to_inorder() == crosstab.base);
                if !base_exists {
                    if let Ok(base_crosstab) =
                        CrossTab::new(&crosstab.base, &crosstab.base, "ALL", "NOPRINT")
                    {
                        additional_crosstabs.push(base_crosstab);
                    }
                }
            }
        }

        // Add the additional crosstabs at the beginning
        for crosstab in additional_crosstabs.into_iter().rev() {
            self.crosstabs.insert(0, crosstab);
        }
    }
}

#[derive(Debug)]
pub struct BannersTable {
    pub index: String,
    pub title: String,
    pub banners: Vec<Banner>,
}

impl BannersTable {
    #[must_use]
    pub fn new(index: &str, title: &str) -> Self {
        let cleaned_index = index.chars().filter(char::is_ascii_digit).collect();

        Self {
            index: cleaned_index,
            title: title.to_string(),
            banners: Vec::new(),
        }
    }

    /// Add a banner to the table.
    ///
    /// # Errors
    ///
    /// Returns `CrossTabsError::ParseError` if the specs cannot be parsed.
    pub fn add_banner(
        &mut self,
        title: &str,
        subtitle: &str,
        specs: &str,
        base: &str,
        options: &str,
    ) -> Result<(), CrossTabsError> {
        let banner = Banner::new(title, subtitle, specs, base, options)?;
        self.banners.push(banner);
        Ok(())
    }
}

#[derive(Debug)]
pub struct BannersTables {
    pub tables: Vec<BannersTable>,
}

impl BannersTables {
    /// Load banner tables from an Excel file.
    ///
    /// # Errors
    ///
    /// Returns `CrossTabsError::ExcelError` if the file cannot be read or parsed.
    pub fn from_excel(filename: &str) -> Result<Self, CrossTabsError> {
        let mut workbook = open_workbook_auto(filename)
            .map_err(|e| CrossTabsError::ExcelError(format!("Failed to open Excel file: {e}")))?;

        let sheet_names = workbook.sheet_names();
        if sheet_names.is_empty() {
            return Err(CrossTabsError::ExcelError(
                "No sheets found in Excel file".to_string(),
            ));
        }

        let range = workbook
            .worksheet_range(&sheet_names[0])
            .map_err(|e| CrossTabsError::ExcelError(format!("Failed to read sheet: {e}")))?;

        let mut tables = Vec::new();
        let mut current_table: Option<BannersTable> = None;
        let mut age_question = String::new();

        for (row_idx, row) in range.rows().enumerate() {
            if row.len() < 4 {
                eprintln!(
                    "Row {} has {} columns (need at least 4)",
                    row_idx,
                    row.len()
                );
                continue;
            }

            let index = Self::cell_to_string(&row[0]);
            let title = Self::cell_to_string(&row[1]);
            let subtitle = Self::cell_to_string(&row[2]);
            let mut specs = Self::cell_to_string(&row[3]);

            // Fix common spec formatting issues
            specs = Self::fix_spec_format(&specs);
            let base = if row.len() >= 5 {
                Self::cell_to_string(&row[4])
            } else {
                "ALL".to_string()
            };

            Self::fix_agegroups(&mut specs, &title, &subtitle, &mut age_question);

            // Check if this starts a new table
            if index.chars().any(|c| !c.is_ascii_digit()) || title == "TITLE" {
                if let Some(table) = current_table.take() {
                    tables.push(table);
                }
                let cleaned_index = index
                    .chars()
                    .filter(char::is_ascii_digit)
                    .collect::<String>();
                current_table = Some(BannersTable::new(
                    &cleaned_index,
                    &format!("/BANNER {cleaned_index}"),
                ));
            } else if !subtitle.is_empty() {
                if let Some(ref mut table) = current_table {
                    match table.add_banner(&title, &subtitle, &specs, &base, "") {
                        Ok(()) => {}
                        Err(e) => {
                            eprintln!(
                                "Warning: Failed to parse specs '{specs}' for banner '{subtitle}': {e}"
                            );
                            // Create a banner with the raw specs as a fallback
                            let fallback_banner = Banner {
                                title: title.trim().to_string(),
                                subtitle: subtitle.trim().to_string(),
                                specs: CrossTabsLogic {
                                    root: CrossTabsNode::Literal(format!("ERROR_PARSING: {specs}")),
                                },
                                base,
                                options: String::new(),
                            };
                            table.banners.push(fallback_banner);
                        }
                    }
                }
            }
        }

        if let Some(table) = current_table {
            tables.push(table);
        }

        Ok(Self { tables })
    }

    fn fix_agegroups(specs: &mut String, title: &str, subtitle: &str, age_question: &mut String) {
        // Handle AGE question special case
        if title == "AGE" {
            age_question.clone_from(specs);
        }

        // Handle age group substitutions
        if subtitle.contains("18-34") {
            *specs = specs.replace(&*age_question, "AGEGROUP:1-2");
        } else if subtitle.contains("18-44") {
            *specs = specs.replace(&*age_question, "AGEGROUP:1-3");
        } else if subtitle.contains("18-54") {
            *specs = specs.replace(&*age_question, "AGEGROUP:1-4");
        } else if subtitle.contains("35-44") {
            *specs = specs.replace(&*age_question, "AGEGROUP:3");
        } else if subtitle.contains("35-54") {
            *specs = specs.replace(&*age_question, "AGEGROUP:3-4");
        } else if subtitle.contains("45-54") {
            *specs = specs.replace(&*age_question, "AGEGROUP:4");
        } else if subtitle.contains("45-64") {
            *specs = specs.replace(&*age_question, "AGEGROUP:4-5");
        } else if subtitle.contains("45+") {
            *specs = specs.replace(&*age_question, "AGEGROUP:4-6");
        } else if subtitle.contains("55-64") {
            *specs = specs.replace(&*age_question, "AGEGROUP:5");
        } else if subtitle.contains("55+") {
            *specs = specs.replace(&*age_question, "AGEGROUP:5-6");
        } else if subtitle.contains("65+") {
            *specs = specs.replace(&*age_question, "AGEGROUP:6");
        } else if subtitle.contains("18-49") {
            *specs = specs.replace(&*age_question, "(QAGE:18-49 OR AGEGROUP:1-3)");
        } else if subtitle.contains("50+") {
            *specs = specs.replace(&*age_question, "(QAGE:50-110 OR AGEGROUP:5-6)");
        }
    }

    fn fix_spec_format(spec: &str) -> String {
        let mut result = spec.to_string();

        // Fix patterns like D2B2 -> D2B:2, Q1A1 -> Q1A:1, etc.
        let fix_pattern = Regex::new(r"([A-Za-z]+\d+[A-Za-z]?)(\d+)$").unwrap();
        if let Some(captures) = fix_pattern.captures(&result) {
            if let (Some(prefix), Some(number)) = (captures.get(1), captures.get(2)) {
                result = format!("{}:{}", prefix.as_str(), number.as_str());
            }
        }

        result
    }

    fn cell_to_string(cell: &Data) -> String {
        match cell {
            Data::String(s) => s.clone(),
            Data::Float(f) => f.to_string(),
            Data::Int(i) => i.to_string(),
            Data::Bool(b) => b.to_string(),
            Data::Empty | Data::Error(_) => String::new(),
            _ => format!("{cell:?}"),
        }
    }

    /// Generate banner output for Uncle syntax.
    ///
    /// # Errors
    ///
    /// Returns `CrossTabsError` if banner generation fails.
    pub fn generate_banner_output(
        &self,
        questions: &HashMap<String, RflQuestion>,
        footer_type: &str,
    ) -> Result<String, CrossTabsError> {
        let mut output = String::new();

        for (table_index, table) in self.tables.iter().enumerate() {
            let table_num = 901 + table_index;

            output.push_str("*\n");
            writeln!(output, "TABLE {table_num}").unwrap();
            writeln!(output, "T /BANNER {}", table.index).unwrap();
            output.push_str("T ");

            let mut col = 2;
            let mut underlines = String::new();
            let mut current_title = String::new();

            // Generate column headers and underlines
            for banner in &table.banners {
                let title_check = banner.uncle_title();
                if !title_check.is_empty() {
                    if !current_title.is_empty() {
                        write!(output, "{} {}", col - 1, current_title).unwrap();
                        write!(underlines, "{}==", col - 1).unwrap();
                    }
                    current_title = title_check;
                    write!(output, "&CC{col}:").unwrap();
                    write!(underlines, "&CC{col}:").unwrap();
                }
                col += 1;
            }

            writeln!(output, "{} {}", col - 1, current_title).unwrap();
            write!(underlines, "{}==", col - 1).unwrap();
            writeln!(output, "T {underlines}").unwrap();

            // Determine format based on number of columns
            let format = if table.banners.len() >= 22 {
                let bleed = if table.banners.len() >= 23 {
                    " BLEED"
                } else {
                    ""
                };
                format!("O FORMAT 27 5 1 0 PDP 0 PCTS ZCELL '-' ZPACELL '-' SZR{bleed}\n")
            } else {
                "O FORMAT 27 6 1 0 PDP 0 PCTS ZCELL '-' ZPACELL '-' SZR\n".to_string()
            };

            output.push_str(&format);
            output.push_str("C &CETOTAL;ALL;COLW 5\n");

            // Generate banner columns
            for banner in &table.banners {
                let subtitle = banner.uncle_subtitle();
                let uncle_syntax = banner.specs.to_uncle_syntax(questions);
                writeln!(output, "C &CE{subtitle};{uncle_syntax}").unwrap();
            }

            // Add footer
            let footer = match footer_type.to_lowercase().as_str() {
                "nbc" => "F &CP HART RESEARCH ASSOCIATES//PUBLIC OPINION STRATEGIES\n",
                "nmb" => "F &CP N M B  R E S E A R C H\n",
                "nbs" => "F &CP N E W   B R I D G E   S T R A T E G Y\n",
                "r2r" => "F &CP R 2 R  R E S E A R C H\n",
                _ => "F &CP P U B L I C   O P I N I O N   S T R A T E G I E S\n",
            };
            output.push_str(footer);
        }

        Ok(output)
    }
}
