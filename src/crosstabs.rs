use crate::rfl::RflQuestion;
use calamine::{open_workbook_auto, Data, Reader};
use regex::Regex;
use std::collections::HashMap;
use std::error::Error;
use std::fmt;

#[derive(Debug, Clone)]
pub enum CrossTabsError {
    ParseError(String),
    ExcelError(String),
    QuestionNotFound(String),
}

impl fmt::Display for CrossTabsError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            CrossTabsError::ParseError(msg) => write!(f, "Parse error: {}", msg),
            CrossTabsError::ExcelError(msg) => write!(f, "Excel error: {}", msg),
            CrossTabsError::QuestionNotFound(msg) => write!(f, "Question not found: {}", msg),
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
    pub fn new(logic: &str) -> Result<Self, CrossTabsError> {
        let processed = Self::preprocess_logic(logic);
        let root = Self::parse_expression(&processed)?;
        Ok(CrossTabsLogic { root })
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
            let inner = &trimmed[1..trimmed.len()-1];
            let inner_node = Self::parse_expression(inner)?;
            return Ok(CrossTabsNode::Group(Box::new(inner_node)));
        }

        // Handle question:code patterns
        if let Some(colon_pos) = trimmed.find(':') {
            let question = trimmed[..colon_pos].trim().to_string();
            let codes = trimmed[colon_pos + 1..].trim().to_string();
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
            
            if paren_depth == 0 {
                if chars[i..].starts_with(&op_chars) {
                    return Some(i);
                }
            }
        }
        None
    }

    pub fn to_inorder(&self) -> String {
        Self::node_to_inorder(&self.root)
    }

    fn node_to_inorder(node: &CrossTabsNode) -> String {
        match node {
            CrossTabsNode::Binary { left, operator, right } => {
                let left_str = Self::node_to_inorder(left);
                let right_str = Self::node_to_inorder(right);
                let op_str = match operator.as_str() {
                    "--" => " MINUS ",
                    "&" => " & ",
                    "|" => " OR ",
                    _ => &format!(" {} ", operator),
                };
                format!("{}{}{}", left_str, op_str, right_str)
            }
            CrossTabsNode::Unary { operator, operand } => {
                format!("{}{}", operator, Self::node_to_inorder(operand))
            }
            CrossTabsNode::Comparison { question, codes } => {
                format!("{}:{}", question, codes)
            }
            CrossTabsNode::Literal(value) => value.clone(),
            CrossTabsNode::Group(inner) => {
                format!("({})", Self::node_to_inorder(inner))
            }
        }
    }

    pub fn to_uncle_syntax(&self, questions: &HashMap<String, RflQuestion>) -> Result<String, CrossTabsError> {
        Self::node_to_uncle_syntax(&self.root, questions)
    }

    fn node_to_uncle_syntax(node: &CrossTabsNode, questions: &HashMap<String, RflQuestion>) -> Result<String, CrossTabsError> {
        match node {
            CrossTabsNode::Binary { left, operator, right } => {
                let left_str = Self::node_to_uncle_syntax(left, questions)
                    .unwrap_or_else(|e| format!("ERROR({})", e));
                let right_str = Self::node_to_uncle_syntax(right, questions)
                    .unwrap_or_else(|e| format!("ERROR({})", e));
                let op_str = match operator.as_str() {
                    "--" => "-",
                    "&" => " ",
                    "|" => " OR ",
                    _ => &format!(" {} ", operator),
                };
                Ok(format!("{}{}{}", left_str, op_str, right_str))
            }
            CrossTabsNode::Unary { operator, operand } => {
                let operand_str = Self::node_to_uncle_syntax(operand, questions)
                    .unwrap_or_else(|e| format!("ERROR({})", e));
                Ok(format!("{}{}", operator, operand_str))
            }
            CrossTabsNode::Comparison { question, codes } => {
                Self::comparison_to_uncle_syntax(question, codes, questions)
                    .or_else(|_| Ok(format!("{}:{}", question, codes))) // Fallback to raw format
            }
            CrossTabsNode::Literal(value) => {
                // Handle special age question case
                if value == "QD1" {
                    // QD1 represents the age question - convert to age range syntax
                    // Based on the Excel age categories: 18-34, 35-44, 45-54, 55-64, 65+
                    // This should map to the AGEGROUP ranges 1-6
                    if let Some(age_question) = questions.get("QD1") {
                        Ok(Self::generate_uncle_syntax(age_question, "1:6"))
                    } else {
                        Ok(value.clone())
                    }
                } else {
                    Ok(value.clone())
                }
            }
            CrossTabsNode::Group(inner) => {
                let inner_str = Self::node_to_uncle_syntax(inner, questions)
                    .unwrap_or_else(|e| format!("ERROR({})", e));
                Ok(format!("({})", inner_str))
            }
        }
    }

    fn comparison_to_uncle_syntax(question_name: &str, codes: &str, questions: &HashMap<String, RflQuestion>) -> Result<String, CrossTabsError> {
        // Try to find the question with or without Q prefix
        let question = questions.get(question_name.to_uppercase().as_str())
            .or_else(|| questions.get(&format!("Q{}", question_name.to_uppercase())))
            .ok_or_else(|| CrossTabsError::QuestionNotFound(format!("Question {} not found in RFL file", question_name)))?;

        let mut processed_codes = codes.replace('-', ":");
        
        // Handle MEAN special case
        if codes == "MEAN" {
            let start_col = question.start_col;
            let end_col = start_col + question.width - 1;
            let mean_part = format!("A(1!{}:{}), ", start_col, end_col);
            processed_codes = format!("{}:{}", question.min_value.unwrap_or(0), question.max_value.unwrap_or(99));
            return Ok(format!("{}{}", mean_part, Self::generate_uncle_syntax(question, &processed_codes)));
        }

        Ok(Self::generate_uncle_syntax(question, &processed_codes))
    }

    fn generate_uncle_syntax(question: &RflQuestion, codes: &str) -> String {
        let start_col = question.start_col;
        let width = question.width;
        let max_responses = question.max_responses;

        if width == 1 {
            if max_responses == 1 {
                format!("1!{}-{}", start_col, codes)
            } else {
                format!("1!{}:{}-{}", start_col, start_col + max_responses - 1, codes)
            }
        } else {
            if max_responses == 1 {
                let end_col = start_col + width - 1;
                format!("R(1!{}:{},{})", start_col, end_col, codes)
            } else {
                let mut ranges = Vec::new();
                for i in 0..max_responses {
                    let range_start = start_col + (i * width);
                    let range_end = range_start + width - 1;
                    ranges.push(format!("1!{}:{}", range_start, range_end));
                }
                format!("R({},{})", ranges.join("/"), codes)
            }
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
    pub fn new(title: String, specs: String, base: String, options: String) -> Result<Self, CrossTabsError> {
        let mut processed_options = options;
        if specs.to_uppercase().contains("MEAN") {
            processed_options = format!("NOVP FDP 0 {}", processed_options);
        }

        Ok(CrossTab {
            title: title.trim().to_string(),
            specs: CrossTabsLogic::new(&specs)?,
            base,
            options: processed_options,
        })
    }

    pub fn uncle_title(&self) -> String {
        self.title.to_uppercase()
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
    pub fn new(title: String, subtitle: String, specs: String, base: String, options: String) -> Result<Self, CrossTabsError> {
        Ok(Banner {
            title: title.trim().to_string(),
            subtitle: subtitle.trim().to_string(),
            specs: CrossTabsLogic::new(&specs)?,
            base,
            options,
        })
    }

    pub fn uncle_title(&self) -> String {
        self.title.to_uppercase()
            .replace('/', "//")
            .replace('=', "==")
    }

    pub fn uncle_subtitle(&self) -> String {
        self.subtitle.to_uppercase()
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
    pub fn new(index: String, title: String, include_text: bool) -> Self {
        let cleaned_index = index.chars().filter(|c| c.is_ascii_digit()).collect();
        
        CrossTabsTable {
            index: cleaned_index,
            title,
            crosstabs: Vec::new(),
            include_text,
        }
    }

    pub fn add_crosstab(&mut self, title: String, specs: String, base: String, options: String) -> Result<(), CrossTabsError> {
        let crosstab = CrossTab::new(title, specs, base, options)?;
        self.crosstabs.push(crosstab);
        Ok(())
    }

    pub fn check_bases(&mut self) {
        let mut additional_crosstabs = Vec::new();
        
        for crosstab in &self.crosstabs {
            if crosstab.base != "ALL" {
                let base_exists = self.crosstabs.iter().any(|ct| ct.specs.to_inorder() == crosstab.base);
                if !base_exists {
                    if let Ok(base_crosstab) = CrossTab::new(
                        crosstab.base.clone(),
                        crosstab.base.clone(),
                        "ALL".to_string(),
                        "NOPRINT".to_string()
                    ) {
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
    pub fn new(index: String, title: String) -> Self {
        let cleaned_index = index.chars().filter(|c| c.is_ascii_digit()).collect();
        
        BannersTable {
            index: cleaned_index,
            title,
            banners: Vec::new(),
        }
    }

    pub fn add_banner(&mut self, title: String, subtitle: String, specs: String, base: String, options: String) -> Result<(), CrossTabsError> {
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
    pub fn from_excel(filename: &str) -> Result<Self, CrossTabsError> {
        let mut workbook = open_workbook_auto(filename)
            .map_err(|e| CrossTabsError::ExcelError(format!("Failed to open Excel file: {}", e)))?;

        let sheet_names = workbook.sheet_names().to_owned();
        if sheet_names.is_empty() {
            return Err(CrossTabsError::ExcelError("No sheets found in Excel file".to_string()));
        }

        let range = workbook.worksheet_range(&sheet_names[0])
            .map_err(|e| CrossTabsError::ExcelError(format!("Failed to read sheet: {}", e)))?;

        let mut tables = Vec::new();
        let mut current_table: Option<BannersTable> = None;
        let mut age_question = String::new();

        for (row_idx, row) in range.rows().enumerate() {
            if row.len() < 4 {
                eprintln!("Row {} has {} columns (need at least 4)", row_idx, row.len());
                continue;
            }

            let index = Self::cell_to_string(&row[0]);
            let title = Self::cell_to_string(&row[1]);
            let subtitle = Self::cell_to_string(&row[2]);
            let mut specs = Self::cell_to_string(&row[3]);
            
            // Fix common spec formatting issues
            specs = Self::fix_spec_format(&specs);
            let base = if row.len() >= 5 { Self::cell_to_string(&row[4]) } else { "ALL".to_string() };
            

            // Handle AGE question special case
            if title == "AGE" {
                age_question = specs.clone();
            }

            // Handle age group substitutions
            if subtitle.contains("18-34") {
                specs = specs.replace(&age_question, "AGEGROUP:1-2");
            } else if subtitle.contains("35-44") {
                specs = specs.replace(&age_question, "AGEGROUP:3");
            } else if subtitle.contains("45-54") {
                specs = specs.replace(&age_question, "AGEGROUP:4");
            } else if subtitle.contains("55-64") {
                specs = specs.replace(&age_question, "AGEGROUP:5");
            } else if subtitle.contains("65+") {
                specs = specs.replace(&age_question, "AGEGROUP:6");
            }

            // Check if this starts a new table
            if index.chars().any(|c| !c.is_ascii_digit()) || title == "TITLE" {
                if let Some(table) = current_table.take() {
                    tables.push(table);
                }
                let cleaned_index = index.chars().filter(|c| c.is_ascii_digit()).collect::<String>();
                current_table = Some(BannersTable::new(cleaned_index.clone(), format!("/BANNER {}", cleaned_index)));
            } else if !subtitle.is_empty() {
                if let Some(ref mut table) = current_table {
                    match table.add_banner(title.clone(), subtitle.clone(), specs.clone(), base.clone(), String::new()) {
                        Ok(()) => {},
                        Err(e) => {
                            eprintln!("Warning: Failed to parse specs '{}' for banner '{}': {}", specs, subtitle, e);
                            // Create a banner with the raw specs as a fallback
                            let fallback_banner = Banner {
                                title: title.trim().to_string(),
                                subtitle: subtitle.trim().to_string(),
                                specs: CrossTabsLogic { 
                                    root: CrossTabsNode::Literal(format!("ERROR_PARSING: {}", specs))
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

        Ok(BannersTables { tables })
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
            Data::Empty => String::new(),
            Data::Error(_) => String::new(),
            _ => format!("{:?}", cell),
        }
    }

    pub fn generate_banner_output(&self, questions: &HashMap<String, RflQuestion>, footer_type: &str) -> Result<String, CrossTabsError> {
        let mut output = String::new();
        
        for (table_index, table) in self.tables.iter().enumerate() {
            let table_num = 901 + table_index;
            
            output.push_str("*\n");
            output.push_str(&format!("TABLE {}\n", table_num));
            output.push_str(&format!("T /BANNER {}\n", table.index));
            output.push_str("T ");
            
            let mut col = 2;
            let mut underlines = String::new();
            let mut current_title = String::new();
            
            // Generate column headers and underlines
            for banner in &table.banners {
                let title_check = banner.uncle_title();
                if !title_check.is_empty() {
                    if !current_title.is_empty() {
                        output.push_str(&format!("{} {}", col - 1, current_title));
                        underlines.push_str(&format!("{}==", col - 1));
                    }
                    current_title = title_check;
                    output.push_str(&format!("&CC{}:", col));
                    underlines.push_str(&format!("&CC{}:", col));
                }
                col += 1;
            }
            
            output.push_str(&format!("{} {}\n", col - 1, current_title));
            underlines.push_str(&format!("{}==", col - 1));
            output.push_str(&format!("T {}\n", underlines));
            
            // Determine format based on number of columns
            let format = if table.banners.len() >= 22 {
                let bleed = if table.banners.len() >= 23 { " BLEED" } else { "" };
                format!("O FORMAT 27 5 1 0 PDP 0 PCTS ZCELL '-' ZPACELL '-' SZR{}\n", bleed)
            } else {
                "O FORMAT 27 6 1 0 PDP 0 PCTS ZCELL '-' ZPACELL '-' SZR\n".to_string()
            };
            
            output.push_str(&format);
            output.push_str("C &CETOTAL;ALL;COLW 5\n");
            
            // Generate banner columns
            for banner in &table.banners {
                let subtitle = banner.uncle_subtitle();
                let uncle_syntax = match banner.specs.to_uncle_syntax(questions) {
                    Ok(syntax) => syntax,
                    Err(e) => {
                        eprintln!("Warning: Failed to generate Uncle syntax for '{}': {}", subtitle, e);
                        format!("ERROR_SYNTAX: {}", banner.specs.to_inorder())
                    }
                };
                output.push_str(&format!("C &CE{};{}\n", subtitle, uncle_syntax));
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
