use ahash::AHashMap;
use crate::rfl::RflQuestion;
use calamine::{open_workbook_auto, Data, Reader};
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
pub struct CrossTabsLogic {
    pub value: Option<String>,
    pub left: Option<Box<Self>>,
    pub right: Option<Box<Self>>,
}

impl CrossTabsLogic {
    fn convert_do_not_select(logic: &str) -> Option<String> {
        use regex::Regex;

        // Pattern: "Q5: DO NOT SELECT :1 OR :3" -> "NOT(Q5:1,3)"
        let re = Regex::new(r"(\w+):\s*DO NOT SELECT\s*:([0-9]+)").ok()?;

        if let Some(caps) = re.captures(logic) {
            let question = caps.get(1)?.as_str();
            let mut codes = vec![caps.get(2)?.as_str()];

            // Find all additional OR'ed codes in the entire string
            let or_re = Regex::new(r"\s+OR\s*:([0-9]+)").ok()?;

            for or_cap in or_re.captures_iter(logic) {
                codes.push(or_cap.get(1)?.as_str());
            }

            return Some(format!("NOT({}:{})", question, codes.join(",")));
        }

        None
    }

    /// Create a new `CrossTabsLogic` instance from a logic expression.
    ///
    /// # Errors
    ///
    /// This function currently does not return errors but may in future versions.
    pub fn new(logic: &str) -> Result<Self, CrossTabsError> {
        let mut logic = logic.to_string();

        // Handle slash syntax: QX2/Q17:1 becomes QX2:1 OR Q17:1
        if logic.contains('/') {
            logic = logic
                .replace("/(", ":(")
                .replace("/Q", ":Q")
                .replace('/', " OR ");
            if let Some(colon_pos) = logic.find(':')
                && let Some(or_pos) = logic[colon_pos..].find(" OR ") {
                    let actual_or_pos = colon_pos + or_pos;
                    let codes = &logic[colon_pos + 1..actual_or_pos];
                    logic = logic.replace(" OR ", &format!("{codes} OR "));
                }
        }
        // Handle "DO NOT SELECT" patterns: "Q5: DO NOT SELECT :1 OR :3" -> "NOT(Q5:1,3)"
        if logic.contains("DO NOT SELECT")
            && let Some(converted) = Self::convert_do_not_select(&logic) {
                logic = converted;
            }

        // Handle "):" syntax...
        if logic.contains("):") {
            logic = logic.replace("):", ":") + ")";
        }

        Ok(Self::parse_simple(&logic))
    }

    fn parse_simple(logic: &str) -> Self {
        let mut logic = logic.trim().to_string();

        // Find operators outside of parentheses (respecting precedence)

        // Check for NOT (highest precedence)
        if logic.starts_with("NOT") {
            return Self {
                value: Some("NOT".to_string()),
                right: Some(Box::new(Self::parse_simple(&logic[3..logic.len()]))),
                left: None,
            };
        }

        // Check for MINUS (highest precedence)
        if let Some(operator_pos) = Self::find_operator_outside_parens(&logic, " MINUS ") {
            return Self {
                value: Some("--".to_string()),
                left: Some(Box::new(Self::parse_simple(&logic[..operator_pos]))),
                right: Some(Box::new(Self::parse_simple(&logic[operator_pos + 7..]))),
            };
        }

        // Check for & or AND
        if let Some(operator_pos) = Self::find_operator_outside_parens(&logic, " & ") {
            return Self {
                value: Some("&".to_string()),
                left: Some(Box::new(Self::parse_simple(&logic[..operator_pos]))),
                right: Some(Box::new(Self::parse_simple(&logic[operator_pos + 3..]))),
            };
        }

        if let Some(operator_pos) = Self::find_operator_outside_parens(&logic, " AND ") {
            return Self {
                value: Some("&".to_string()),
                left: Some(Box::new(Self::parse_simple(&logic[..operator_pos]))),
                right: Some(Box::new(Self::parse_simple(&logic[operator_pos + 5..]))),
            };
        }

        // Check for OR (lowest precedence)
        if let Some(operator_pos) = Self::find_operator_outside_parens(&logic, " OR ") {
            return Self {
                value: Some("|".to_string()),
                left: Some(Box::new(Self::parse_simple(&logic[..operator_pos]))),
                right: Some(Box::new(Self::parse_simple(&logic[operator_pos + 4..]))),
            };
        }

        // Handle parentheses with colon content (like Perl: m/(\().*?:/)
        if logic.starts_with('(') && logic.contains(':') {
            return Self {
                value: Some("(".to_string()),
                left: Some(Box::new(Self::parse_simple(&logic[1..logic.len() - 1]))),
                right: None,
            };
        }

        // Handle balanced parentheses
        if logic.starts_with('(') && logic.ends_with(')') && Self::is_balanced_parentheses(&logic) {
            return Self {
                value: Some("(".to_string()),
                left: Some(Box::new(Self::parse_simple(&logic[1..logic.len() - 1]))),
                right: None,
            };
        }

        // Handle colon comparison
        if logic.contains(':') {
            return Self {
                value: Some(":".to_string()),
                left: Some(Box::new(Self::parse_simple(
                    &logic[..logic.find(':').unwrap()],
                ))),
                right: Some(Box::new(Self::parse_simple(
                    &logic[logic.find(':').unwrap() + 1..],
                ))),
            };
        }

        // Leaf node - remove whitespace for consistency with Perl
        logic = logic.replace(", ", ",");
        Self {
            value: Some(logic),
            left: None,
            right: None,
        }
    }

    const fn is_leaf(&self) -> bool {
        self.left.is_none() && self.right.is_none()
    }

    fn find_operator_outside_parens(logic: &str, operator: &str) -> Option<usize> {
        let mut paren_depth = 0;
        let logic_chars: Vec<char> = logic.chars().collect();
        let op_chars: Vec<char> = operator.chars().collect();

        for i in 0..=logic_chars.len().saturating_sub(op_chars.len()) {
            match logic_chars[i] {
                '(' => paren_depth += 1,
                ')' => paren_depth -= 1,
                _ => {}
            }

            if paren_depth == 0 && logic_chars[i..].starts_with(&op_chars) {
                return Some(i);
            }
        }
        None
    }

    fn is_balanced_parentheses(logic: &str) -> bool {
        let mut depth = 0;
        for c in logic.chars() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth < 0 {
                        return false;
                    }
                    // If we reach 0 before the end, the outer parens don't wrap everything
                    if depth == 0 && c != logic.chars().last().unwrap_or(' ') {
                        return false;
                    }
                }
                _ => {}
            }
        }
        depth == 0
    }

    #[must_use]
    #[allow(clippy::option_if_let_else)]
    pub fn to_inorder(&self) -> String {
        if let Some(ref left) = self.left {
            let left_str = left.to_inorder();
            if let Some(ref value) = self.value {
                match value.as_str() {
                    "--" => {
                        if let Some(ref right) = self.right {
                            format!("{} MINUS {}", left_str, right.to_inorder())
                        } else {
                            left_str
                        }
                    }
                    "&" => {
                        if let Some(ref right) = self.right {
                            format!("{} & {}", left_str, right.to_inorder())
                        } else {
                            left_str
                        }
                    }
                    "|" => {
                        if let Some(ref right) = self.right {
                            format!("{} OR {}", left_str, right.to_inorder())
                        } else {
                            left_str
                        }
                    }
                    ":" => {
                        if let Some(ref right) = self.right {
                            format!("{}:{}", left_str, right.to_inorder())
                        } else {
                            left_str
                        }
                    }
                    "(" => {
                        format!("({left_str})")
                    }
                    _ => {
                        if let Some(ref right) = self.right {
                            format!("{} {} {}", left_str, value, right.to_inorder())
                        } else {
                            left_str
                        }
                    }
                }
            } else {
                left_str
            }
        } else if let Some(ref value) = self.value {
            if value.as_str() == "NOT" {
                if let Some(ref right) = self.right {
                    let right_str = right.to_inorder();
                    // Check if the right side already has parentheses or if it should be wrapped
                    if right.value.as_deref() == Some("(") || right_str.starts_with('(') {
                        format!("NOT{right_str}")
                    } else {
                        format!("NOT({right_str})")
                    }
                } else {
                    value.clone()
                }
            } else {
                // If this is a literal string with parentheses, print it as-is (like Perl does)
                if value.contains('(') || value.contains(')') {
                    eprintln!("Don't know what to do with '{value}'");
                }
                value.clone()
            }
        } else {
            String::new()
        }
    }

    /// Convert the logic expression to Uncle syntax.
    #[must_use]
    #[allow(clippy::option_if_let_else)]
    pub fn to_uncle_syntax(&self, questions: &AHashMap<String, RflQuestion>) -> String {
        if let Some(ref value) = self.value {
            if value == ":" {
                self.handle_comparison(questions)
            } else {
                self.handle_operators(questions)
            }
        } else {
            String::new()
        }
    }

    fn handle_comparison(&self, questions: &AHashMap<String, RflQuestion>) -> String {
        if let (Some(left), Some(right)) = (&self.left, &self.right) {
            if !left.is_leaf() || !right.is_leaf() {
                eprintln!(
                    "{}-{} Non leafs being compared.",
                    left.value.as_ref().unwrap_or(&String::new()),
                    right.value.as_ref().unwrap_or(&String::new())
                );
            }

            let question_name = left.value.as_deref().unwrap_or("").to_uppercase();
            let codes = right.value.as_deref().unwrap_or("");

            // Try to find the question with or without Q prefix
            let question = questions
                .get(&question_name)
                .or_else(|| questions.get(&format!("Q{question_name}")));

            if let Some(question) = question {
                let mut punches = codes.replace('-', ":");

                // Handle MEAN special case
                if codes == "MEAN" {
                    let mean_part = format!(
                        "A(1!{}:{}), ",
                        question.start_col,
                        question.start_col + question.width - 1
                    );
                    punches = format!(
                        "{}:{}",
                        question.min_value.unwrap_or(0),
                        question.max_value.unwrap_or(99)
                    );
                    return format!(
                        "{}{}",
                        mean_part,
                        Self::generate_uncle_syntax(question, &punches)
                    );
                }

                Self::generate_uncle_syntax(question, &punches)
            } else {
                eprintln!("{question_name} not found in RFL file.");
                format!("{question_name}:{codes}")
            }
        } else {
            String::new()
        }
    }

    #[allow(clippy::option_if_let_else)]
    fn handle_operators(&self, questions: &AHashMap<String, RflQuestion>) -> String {
        let value = self.value.as_ref().unwrap();

        if let Some(ref left) = self.left {
            let left_str = left.to_uncle_syntax(questions);
            match value.as_str() {
                "--" => {
                    if let Some(ref right) = self.right {
                        format!("{}-{}", left_str, right.to_uncle_syntax(questions))
                    } else {
                        left_str
                    }
                }
                "|" => {
                    if let Some(ref right) = self.right {
                        format!("{} OR {}", left_str, right.to_uncle_syntax(questions))
                    } else {
                        left_str
                    }
                }
                "&" => {
                    if let Some(ref right) = self.right {
                        format!("{} {}", left_str, right.to_uncle_syntax(questions))
                    } else {
                        left_str
                    }
                }
                "(" => {
                    format!("({left_str})")
                }
                _ => {
                    if value.is_empty() {
                        left_str
                    } else {
                        eprintln!("Don't know what to do with '{value}'");
                        if let Some(ref right) = self.right {
                            format!(
                                "{} {} {}",
                                left_str,
                                value,
                                right.to_uncle_syntax(questions)
                            )
                        } else {
                            format!("{left_str}{value}")
                        }
                    }
                }
            }
        } else if let Some(ref right) = self.right {
            match value.as_str() {
                "NOT" => {
                    if right.is_leaf() {
                        String::new()
                    } else {
                        format!("NOT{}", right.to_uncle_syntax(questions))
                    }
                }
                _ => String::new(),
            }
        } else {
            String::new()
        }
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
            format!("R(1!{}:{},{})", start_col, start_col + width - 1, codes)
        } else if max_responses <= 3 {
            let mut ranges = Vec::new();
            for i in 0..max_responses {
                let range_start = start_col + (i * width);
                let range_end = range_start + width - 1;
                ranges.push(format!("1!{range_start}:{range_end}"));
            }
            format!("R({},{})", ranges.join("/"), codes)
        } else {
            let mut ranges = Vec::new();
            for i in 0..max_responses {
                let range_start = start_col + (i * width);
                let range_end = range_start + width - 1;
                if i == 0 {
                    ranges.push(format!("1!{range_start}:{range_end}/"));
                } else if i == 1 {
                    ranges.push(format!("1!{range_start}:{range_end}..."));
                } else if i == max_responses - 1 {
                    ranges.push(format!("1!{range_start}:{range_end}"));
                }
            }
            format!("R({},{})", ranges.join(""), codes)
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
    /// Returns an error if the specs cannot be parsed.
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
    /// Returns an error if the specs cannot be parsed.
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
    /// Returns an error if the crosstab specs cannot be parsed.
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
                if !base_exists
                    && let Ok(base_crosstab) =
                        CrossTab::new(&crosstab.base, &crosstab.base, "ALL", "NOPRINT")
                    {
                        additional_crosstabs.push(base_crosstab);
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
    /// Returns an error if the banner specs cannot be parsed.
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
    /// Returns an error if the Excel file cannot be read or parsed.
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
        let mut group_title = String::new();
        let mut age_title = String::new();
        let mut region_title = String::new();
        let mut region_question = String::new();

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

            group_title = if title.is_empty() {
                group_title
            } else {
                title.clone()
            };
            if group_title == "AGE" || group_title == "AGE II" {
                age_title.clone_from(&title);
            }
            if group_title.contains("REGION") {
                region_title.clone_from(&title);
            }

            // Fix common spec formatting issues
            specs = Self::fix_spec_format(&specs);
            let base = if row.len() >= 5 {
                Self::cell_to_string(&row[4])
            } else {
                "ALL".to_string()
            };

            Self::fix_agegroups(&mut specs, &age_title, &subtitle, &mut age_question);
            Self::fix_national_regions(&mut specs, &region_title, &subtitle, &mut region_question);

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
            } else if !subtitle.is_empty()
                && let Some(ref mut table) = current_table {
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
                                    value: Some(format!("ERROR_PARSING: {specs}")),
                                    left: None,
                                    right: None,
                                },
                                base,
                                options: String::new(),
                            };
                            table.banners.push(fallback_banner);
                        }
                    }
                }
        }

        if let Some(table) = current_table {
            tables.push(table);
        }

        Ok(Self { tables })
    }

    fn fix_agegroups(
        specs: &mut String,
        age_title: &str,
        subtitle: &str,
        age_question: &mut String,
    ) {
        // Handle AGE question special case
        if age_title == "AGE" || age_title == "AGE II" {
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
        } else if subtitle.contains("35-49") {
            *specs = specs.replace(&*age_question, "(QAGE:35-49 OR AGEGROUP:3)");
        } else if subtitle.contains("50-64") {
            *specs = specs.replace(&*age_question, "(QAGE:50-64 OR AGEGROUP:5)");
        }
    }

    fn fix_national_regions(
        specs: &mut String,
        region_title: &str,
        subtitle: &str,
        region_question: &mut String,
    ) {
        // Handle AGE question special case
        if region_title.contains("REGION") {
            region_question.clone_from(specs);
        }

        // Handle age group substitutions
        if subtitle.contains("NORTHEAST") {
            *specs = specs.replace(
                &*region_question,
                "FIPSCOMB:09,23,25,33,44,50,10,11,24,34,36,42,54",
            );
        } else if subtitle.contains("MIDWEST") {
            *specs = specs.replace(
                &*region_question,
                "FIPSCOMB:17,18,26,27,39,55,19,20,29,31,38,46",
            );
        } else if subtitle.contains("SOUTH") {
            *specs = specs.replace(
                &*region_question,
                "FIPSCOMB:01,05,12,13,22,28,45,21,37,40,47,48,51",
            );
        } else if subtitle.contains("WEST") {
            *specs = specs.replace(
                &*region_question,
                "FIPSCOMB:02,04,08,16,30,32,35,49,56,06,15,41,53",
            );
        } else if subtitle.contains("NEW ENGLAND") {
            *specs = specs.replace(&*region_question, "FIPSCOMB:09,23,25,33,44,50");
        } else if subtitle.contains("MID-ATLANTIC") {
            *specs = specs.replace(&*region_question, "FIPSCOMB:10,11,24,34,36,42,54");
        } else if subtitle.contains("GREAT LAKES") {
            *specs = specs.replace(&*region_question, "FIPSCOMB:17,18,26,27,39,55");
        } else if subtitle.contains("FARM BELT") {
            *specs = specs.replace(&*region_question, "FIPSCOMB:19,20,29,31,38,46");
        } else if subtitle.contains("DEEP SOUTH") {
            *specs = specs.replace(&*region_question, "FIPSCOMB:01,05,12,13,22,28,45");
        } else if subtitle.contains("OUTER SOUTH") {
            *specs = specs.replace(&*region_question, "FIPSCOMB:21,37,40,47,48,51");
        } else if subtitle.contains("MOUNTAIN") {
            *specs = specs.replace(&*region_question, "FIPSCOMB:04,08,16,30,32,35,49,56");
        } else if subtitle.contains("PACIFIC") {
            *specs = specs.replace(&*region_question, "FIPSCOMB:02,06,15,41,53");
        }
    }

    fn fix_spec_format(spec: &str) -> String {
        // Fix patterns like D2B2 -> D2B:2, Q1A1 -> Q1A:1, etc.
        // Simple pattern matching without regex for performance
        let spec = spec.trim();

        // Look for pattern: letters + digits + letters + digits at end
        let mut last_letter_pos = None;
        let mut digit_start = None;

        for (i, c) in spec.char_indices() {
            if c.is_ascii_alphabetic() {
                if digit_start.is_some() {
                    last_letter_pos = Some(i);
                    digit_start = None;
                }
            } else if c.is_ascii_digit() && last_letter_pos.is_some() && digit_start.is_none() {
                digit_start = Some(i);
            }
        }

        if let (Some(_letter_pos), Some(digit_pos)) = (last_letter_pos, digit_start)
            && (digit_pos == spec.len() - 1
                || spec.chars().skip(digit_pos).all(|c| c.is_ascii_digit()))
            {
                return format!("{}:{}", &spec[..digit_pos], &spec[digit_pos..]);
            }

        spec.to_string()
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
    /// Returns an error if banner generation fails.
    pub fn generate_banner_output(
        &self,
        questions: &AHashMap<String, RflQuestion>,
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
                let bleed = if table.banners.len() >= 24 {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_do_not_select() {
        // Test the specific case that broke
        assert_eq!(
            CrossTabsLogic::convert_do_not_select("Q5: DO NOT SELECT :1 OR :3"),
            Some("NOT(Q5:1,3)".to_string())
        );

        // Test single value
        assert_eq!(
            CrossTabsLogic::convert_do_not_select("Q10: DO NOT SELECT :2"),
            Some("NOT(Q10:2)".to_string())
        );

        // Test multiple OR values
        assert_eq!(
            CrossTabsLogic::convert_do_not_select("QX1: DO NOT SELECT :1 OR :2 OR :5"),
            Some("NOT(QX1:1,2,5)".to_string())
        );

        // Test with extra whitespace
        assert_eq!(
            CrossTabsLogic::convert_do_not_select("Q5:    DO NOT SELECT   :1   OR   :3"),
            Some("NOT(Q5:1,3)".to_string())
        );

        // Test non-matching cases
        assert_eq!(
            CrossTabsLogic::convert_do_not_select("Q5: SELECT :1 OR :3"),
            None
        );
    }

    #[test]
    fn test_do_not_select_integration() {
        // Test that the full parsing works
        let result = CrossTabsLogic::new("Q5: DO NOT SELECT :1 OR :3").unwrap();

        // Now that we fixed the NOT handling, the to_inorder should work correctly
        let converted = result.to_inorder();
        assert_eq!(converted, "NOT(Q5:1,3)");
    }

    #[test]
    fn test_not_expression_reconstruction() {
        // Test various NOT expressions to ensure proper reconstruction
        let simple_not = CrossTabsLogic::new("NOT(Q1:1)").unwrap();
        assert_eq!(simple_not.to_inorder(), "NOT(Q1:1)");

        let complex_not = CrossTabsLogic::new("NOT(Q1:1 OR Q2:2)").unwrap();
        assert_eq!(complex_not.to_inorder(), "NOT(Q1:1 OR Q2:2)");
    }
}
