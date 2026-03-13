use std::fmt;

use ipf_survey::{
    CodedSurvey, PopulationTargets, RakingConfig, RakingError, RakingResult, ValidatedTargets,
    Variable, rake,
};

use ahash::AHashMap;

use crate::cfmc::CfmcLogic;
use crate::rfl::{RflFile, RflQuestion};
use crate::weight::UncleExpr;

// ---------------------------------------------------------------------------
// Public data structures
// ---------------------------------------------------------------------------

pub struct WeightScheme {
    pub tables: Vec<WeightTable>,
    pub config: WeightConfig,
}

/// Result from [`compute_weights_multi_pass`], including per-pass convergence.
pub struct MultiPassResult {
    /// One weight per record: `Some(w)` if raked, `None` if unqualified.
    pub weights: Vec<Option<f64>>,
    /// Convergence report for each pass that was executed.
    pub pass_results: Vec<RakingResult>,
}

pub struct WeightTable {
    pub id: u16,
    pub label: Option<String>,
    pub categories: Vec<WeightCategory>,
}

pub struct WeightCategory {
    pub label: String,
    pub condition: WeightCondition,
    pub target: f64,
}

#[derive(Clone)]
pub enum WeightCondition {
    Cfmc(CfmcLogic),
    Uncle(UncleExpr),
}

impl WeightCondition {
    pub fn evaluate(
        &self,
        questions: Option<&AHashMap<String, RflQuestion>>,
        record: &str,
    ) -> Result<bool, String> {
        match self {
            Self::Cfmc(logic) => {
                let questions =
                    questions.ok_or("RFL layout file is required to evaluate CFMC conditions")?;
                logic.evaluate(questions, record)
            }
            Self::Uncle(expr) => expr.evaluate(record),
        }
    }
}

pub struct WeightConfig {
    pub raking: RakingConfig,
    /// RFL question label whose numeric value is used as the design weight.
    /// If `None`, all base weights are 1.0.
    pub base_weight_field: Option<String>,
    /// Raw column range (1-based, inclusive) containing a prior weight to use
    /// as the base weight. Parsed from `CWEIGHT(F!col:col)` + `RETAIN` in
    /// `.E` files. Takes precedence over `base_weight_field` when both are set.
    pub base_weight_columns: Option<(usize, usize)>,
    /// Tolerance for grand-total consistency across weight tables.
    /// Real-world tables often have rounded targets that don't sum identically.
    /// Default: `1e-6`. Set higher (e.g. `0.01`) for typical production tables.
    pub target_tolerance: Option<f64>,
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum WeightError {
    /// CFMC evaluation failure on a specific record.
    ConditionEval {
        table_id: u16,
        row_label: String,
        record: usize,
        source: String,
    },

    /// Record matched no category in a table.
    Unclassified { table_id: u16, record: usize },

    /// Record matched multiple categories in a table.
    MultipleMatches {
        table_id: u16,
        record: usize,
        matched: Vec<String>,
    },

    /// Base weight field not found in RFL or value couldn't be parsed.
    BaseWeightField {
        field: String,
        record: usize,
        detail: String,
    },

    /// Target values don't sum consistently across tables, or other target problem.
    TargetMismatch { source: RakingError },

    /// Raking did not converge or another raking-layer failure.
    RakingFailed { source: RakingError },

    /// A weight pass references a table ID that doesn't exist.
    MissingTable { table_id: u16 },
}

impl fmt::Display for WeightError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConditionEval {
                table_id,
                row_label,
                record,
                source,
            } => write!(
                f,
                "table {table_id} row \"{row_label}\" record {record}: condition eval error: {source}"
            ),
            Self::Unclassified { table_id, record } => {
                write!(f, "table {table_id} record {record}: no category matched")
            }
            Self::MultipleMatches {
                table_id,
                record,
                matched,
            } => write!(
                f,
                "table {table_id} record {record}: multiple categories matched: {}",
                matched.join(", ")
            ),
            Self::BaseWeightField {
                field,
                record,
                detail,
            } => write!(f, "base weight field \"{field}\" record {record}: {detail}"),
            Self::TargetMismatch { source } => write!(f, "target validation failed: {source}"),
            Self::RakingFailed { source } => write!(f, "raking failed: {source}"),
            Self::MissingTable { table_id } => {
                write!(f, "weight pass references unknown table ID {table_id}")
            }
        }
    }
}

impl std::error::Error for WeightError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::TargetMismatch { source } | Self::RakingFailed { source } => Some(source),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Classify records and run raking. Returns one weight per record.
pub fn compute_weights(
    scheme: &WeightScheme,
    rfl: Option<&RflFile>,
    data: &[impl AsRef<str>],
) -> Result<Vec<f64>, WeightError> {
    let (survey, targets) = classify(scheme, rfl, data)?;
    rake_classified(&survey, &targets, &scheme.config.raking)
}

/// Compute weights for qualified/split-sample designs.
///
/// Each pass applies to a different subset of records (determined by evaluating
/// `pass.qualifier`) and is raked independently with its own tables and TOTAL.
/// Returns `Option<f64>` per record: `Some(weight)` for records that were
/// raked, `None` for records that didn't match any qualifier.
///
/// **Callers are responsible for ensuring qualifiers are mutually exclusive
/// when records should receive exactly one weight.** If a record matches
/// multiple passes, the last matching pass wins.
///
/// # Errors
///
/// Returns `WeightError` if qualifier evaluation, classification, raking,
/// or table lookup fails for any pass.
pub fn compute_weights_multi_pass(
    passes: &[crate::weight::QualifiedWeightPass],
    tables: &[WeightTable],
    config: &WeightConfig,
    rfl: Option<&RflFile>,
    data: &[impl AsRef<str>],
) -> Result<MultiPassResult, WeightError> {
    let mut weights: Vec<Option<f64>> = vec![None; data.len()];
    let mut pass_results: Vec<RakingResult> = Vec::new();

    for pass in passes {
        // Determine which records qualify for this pass.
        let qualifying: Vec<usize> = if let Some(ref qual) = pass.qualifier {
            data.iter()
                .enumerate()
                .filter_map(|(i, line)| match qual.evaluate(line.as_ref()) {
                    Ok(true) => Some(Ok(i)),
                    Ok(false) => None,
                    Err(e) => Some(Err(WeightError::ConditionEval {
                        table_id: 0,
                        row_label: "X SET QUAL".to_string(),
                        record: i,
                        source: e,
                    })),
                })
                .collect::<Result<Vec<_>, _>>()?
        } else {
            (0..data.len()).collect()
        };

        if qualifying.is_empty() {
            continue;
        }

        // Extract the subset of data for this pass (borrow, no clone).
        let subset: Vec<&str> = qualifying.iter().map(|&i| data[i].as_ref()).collect();

        let normalization = pass.directive.total.map_or(
            ipf_survey::Normalization::SumToN,
            ipf_survey::Normalization::SumTo,
        );

        let pass_config = WeightConfig {
            raking: RakingConfig {
                normalization,
                ..config.raking.clone()
            },
            base_weight_field: config.base_weight_field.clone(),
            base_weight_columns: config.base_weight_columns,
            target_tolerance: config.target_tolerance,
        };

        // Build a temporary scheme from this pass's referenced tables.
        let scheme = WeightScheme {
            tables: pass
                .directive
                .table_ids
                .iter()
                .map(|&id| {
                    let t = tables
                        .iter()
                        .find(|t| t.id == id)
                        .ok_or(WeightError::MissingTable { table_id: id })?;
                    Ok(WeightTable {
                        id: t.id,
                        label: t.label.clone(),
                        categories: t
                            .categories
                            .iter()
                            .map(|c| WeightCategory {
                                label: c.label.clone(),
                                condition: c.condition.clone(),
                                target: c.target,
                            })
                            .collect(),
                    })
                })
                .collect::<Result<Vec<_>, _>>()?,
            config: pass_config,
        };

        let (survey, targets) = classify(&scheme, rfl, &subset)?;
        let result = rake_classified_full(&survey, &targets, &scheme.config.raking)?;

        // Merge back into the main weights vector.
        for (j, &record_idx) in qualifying.iter().enumerate() {
            weights[record_idx] = Some(result.weights[j]);
        }
        pass_results.push(result);
    }

    Ok(MultiPassResult {
        weights,
        pass_results,
    })
}

/// Extract base weights from column positions or an RFL field.
fn extract_base_weights(
    config: &WeightConfig,
    rfl: Option<&RflFile>,
    data: &[impl AsRef<str>],
) -> Result<Option<Vec<f64>>, WeightError> {
    if let Some((col_start, col_end)) = config.base_weight_columns {
        let start_idx = col_start - 1; // 1-based → 0-based
        let label = format!("cols {col_start}:{col_end}");
        let weights: Result<Vec<f64>, WeightError> = data
            .iter()
            .enumerate()
            .map(|(r, line)| {
                let line = line.as_ref();
                let raw = if line.len() >= col_end {
                    line[start_idx..col_end].trim()
                } else if line.len() > start_idx {
                    line[start_idx..].trim()
                } else {
                    ""
                };
                raw.parse::<f64>().map_err(|_| WeightError::BaseWeightField {
                    field: label.clone(),
                    record: r,
                    detail: format!("could not parse \"{raw}\" as f64"),
                })
            })
            .collect();
        Ok(Some(weights?))
    } else if let Some(field) = &config.base_weight_field {
        let rfl = rfl.ok_or_else(|| WeightError::BaseWeightField {
            field: field.clone(),
            record: 0,
            detail: "RFL layout file is required for base_weight_field".to_string(),
        })?;
        let question = rfl
            .get_question(field)
            .ok_or_else(|| WeightError::BaseWeightField {
                field: field.clone(),
                record: 0,
                detail: format!("question \"{field}\" not found in RFL"),
            })?;
        let weights: Result<Vec<f64>, WeightError> = data
            .iter()
            .enumerate()
            .map(|(r, line)| {
                let responses = question.extract_responses(line.as_ref());
                let raw = responses.first().map_or("", |s| s.trim());
                raw.parse::<f64>()
                    .map_err(|_| WeightError::BaseWeightField {
                        field: field.clone(),
                        record: r,
                        detail: format!("could not parse \"{raw}\" as f64"),
                    })
            })
            .collect();
        Ok(Some(weights?))
    } else {
        Ok(None)
    }
}

/// Classify records against a scheme.
/// Returns a `CodedSurvey` and `ValidatedTargets` ready for raking.
pub fn classify(
    scheme: &WeightScheme,
    rfl: Option<&RflFile>,
    data: &[impl AsRef<str>],
) -> Result<(CodedSurvey, ValidatedTargets), WeightError> {
    let n_records = data.len();
    let n_tables = scheme.tables.len();
    let questions = rfl.map(RflFile::questions);

    let base_weights = extract_base_weights(&scheme.config, rfl, data)?;

    // Classify: flat codes array, row-major (record × table).
    let mut codes = vec![0usize; n_records * n_tables];

    for (r, line) in data.iter().enumerate() {
        let line = line.as_ref();
        for (t, table) in scheme.tables.iter().enumerate() {
            let mut matched_indices: Vec<usize> = Vec::new();

            for (c, category) in table.categories.iter().enumerate() {
                let hit = category.condition.evaluate(questions, line).map_err(|e| {
                    WeightError::ConditionEval {
                        table_id: table.id,
                        row_label: category.label.clone(),
                        record: r,
                        source: e,
                    }
                })?;
                if hit {
                    matched_indices.push(c);
                }
            }

            match matched_indices.len() {
                0 => {
                    return Err(WeightError::Unclassified {
                        table_id: table.id,
                        record: r,
                    });
                }
                1 => {
                    codes[r * n_tables + t] = matched_indices[0];
                }
                _ => {
                    let labels = matched_indices
                        .iter()
                        .map(|&c| table.categories[c].label.clone())
                        .collect();
                    return Err(WeightError::MultipleMatches {
                        table_id: table.id,
                        record: r,
                        matched: labels,
                    });
                }
            }
        }
    }

    // Build ipf_survey Variable descriptors — one per weight table.
    let variables: Vec<Variable> = scheme
        .tables
        .iter()
        .map(|t| Variable {
            name: format!("TABLE_{}", t.id),
            levels: t.categories.len(),
            labels: Some(t.categories.iter().map(|c| c.label.clone()).collect()),
        })
        .collect();

    // Construct CodedSurvey.
    let survey = match base_weights {
        Some(bw) => CodedSurvey::from_flat_codes_weighted(variables, codes, n_records, bw),
        None => CodedSurvey::from_flat_codes(variables, codes, n_records),
    }
    .map_err(|e| WeightError::RakingFailed { source: e })?;

    // Build and validate population targets.
    let pt = scheme
        .tables
        .iter()
        .fold(PopulationTargets::new(), |acc, table| {
            acc.add(
                &format!("TABLE_{}", table.id),
                table.categories.iter().map(|c| c.target).collect(),
            )
        });
    let targets = match scheme.config.target_tolerance {
        Some(tol) => pt.validate_with_tolerance(&survey, tol),
        None => pt.validate(&survey),
    }
    .map_err(|e| WeightError::TargetMismatch { source: e })?;

    Ok((survey, targets))
}

/// Rake a pre-classified survey. Returns one weight per record.
pub fn rake_classified(
    survey: &CodedSurvey,
    targets: &ValidatedTargets,
    config: &RakingConfig,
) -> Result<Vec<f64>, WeightError> {
    rake(survey, targets, config)
        .map(|r| r.weights)
        .map_err(|e| WeightError::RakingFailed { source: e })
}

/// Rake a pre-classified survey. Returns the full `RakingResult` including
/// convergence report and diagnostics.
pub fn rake_classified_full(
    survey: &CodedSurvey,
    targets: &ValidatedTargets,
    config: &RakingConfig,
) -> Result<RakingResult, WeightError> {
    rake(survey, targets, config).map_err(|e| WeightError::RakingFailed { source: e })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rfl::RflFile;

    // -----------------------------------------------------------------------
    // Fixtures
    // -----------------------------------------------------------------------

    /// Minimal RFL with GENDER at col 43 (width 1) and AGEGROUP at col 41 (width 1).
    /// Matches the real p0157.rfl layout used in the E file weight tables.
    fn make_rfl() -> RflFile {
        let lines: Vec<String> = [
            "Q GENDER                         FLD  [2994]         --> [43]",
            "T GENDER",
            "R     1                                                             MALE",
            "R     2                                                             FEMALE",
            "R     3                                                             OTHER",
            "Q AGEGROUP                       FLD  [2984]         --> [41]",
            "T AGEGROUP",
            "R     1                                                             18-24",
            "R     2                                                             25-34",
            "R     3                                                             35-44",
            "R     4                                                             45-54",
            "R     5                                                             55-64",
            "R     6                                                             65+",
            "R     7                                                             REFUSED",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        RflFile::from_lines(&lines).unwrap()
    }

    /// Same as `make_rfl` plus a BASEWT numeric field at col 45, width 5.
    fn make_rfl_with_basewt() -> RflFile {
        let lines: Vec<String> = [
            "Q GENDER                         FLD  [2994]         --> [43]",
            "T GENDER",
            "R     1                                                             MALE",
            "R     2                                                             FEMALE",
            "R     3                                                             OTHER",
            "Q AGEGROUP                       FLD  [2984]         --> [41]",
            "T AGEGROUP",
            "R     1                                                             18-24",
            "R     2                                                             25-34",
            "R     3                                                             35-44",
            "R     4                                                             45-54",
            "R     5                                                             55-64",
            "R     6                                                             65+",
            "R     7                                                             REFUSED",
            "Q BASEWT                         NUM  [3000.5]       --> [45.5]",
            "T BASE WEIGHT",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        RflFile::from_lines(&lines).unwrap()
    }

    /// Build a 50-char fixed-width record with specific AGEGROUP (col 41) and GENDER (col 43).
    fn make_record(agegroup: char, gender: char) -> String {
        let mut s = vec![b' '; 50];
        s[40] = agegroup as u8; // col 41 → index 40
        s[42] = gender as u8; // col 43 → index 42
        String::from_utf8(s).unwrap()
    }

    /// Same as `make_record` but also writes a 5-char base weight at col 45 (indices 44–48).
    fn make_record_with_basewt(agegroup: char, gender: char, weight: &str) -> String {
        let mut s = vec![b' '; 50];
        s[40] = agegroup as u8;
        s[42] = gender as u8;
        for (i, b) in weight.bytes().take(5).enumerate() {
            s[44 + i] = b; // col 45 → index 44
        }
        String::from_utf8(s).unwrap()
    }

    /// Standard two-table scheme: TABLE_601 (GENDER) + TABLE_603 (AGEGROUP).
    /// Targets mirror the real E file proportions and sum to 1.0 across both tables.
    fn make_scheme() -> WeightScheme {
        WeightScheme {
            tables: vec![
                WeightTable {
                    id: 601,
                    label: Some("Gender".to_string()),
                    categories: vec![
                        WeightCategory {
                            label: "MALE".to_string(),
                            condition: WeightCondition::Cfmc(
                                CfmcLogic::parse("GENDER(1)").unwrap(),
                            ),
                            target: 0.48,
                        },
                        WeightCategory {
                            label: "FEMALE/OTHER".to_string(),
                            condition: WeightCondition::Cfmc(
                                CfmcLogic::parse("GENDER(2,3)").unwrap(),
                            ),
                            target: 0.52,
                        },
                    ],
                },
                WeightTable {
                    id: 603,
                    label: Some("Age".to_string()),
                    categories: vec![
                        WeightCategory {
                            label: "18-44".to_string(),
                            condition: WeightCondition::Cfmc(
                                CfmcLogic::parse("AGEGROUP(1,2,3)").unwrap(),
                            ),
                            target: 0.47,
                        },
                        WeightCategory {
                            label: "65+".to_string(),
                            condition: WeightCondition::Cfmc(
                                CfmcLogic::parse("AGEGROUP(6)").unwrap(),
                            ),
                            target: 0.20,
                        },
                        WeightCategory {
                            label: "BALANCE".to_string(),
                            condition: WeightCondition::Cfmc(
                                CfmcLogic::parse("AGEGROUP(4,5,7)").unwrap(),
                            ),
                            target: 0.33,
                        },
                    ],
                },
            ],
            config: WeightConfig {
                raking: RakingConfig::default(),
                base_weight_field: None,
                base_weight_columns: None,
                target_tolerance: None,
            },
        }
    }

    // Six records covering all category combinations used in make_scheme().
    fn make_data() -> Vec<String> {
        vec![
            make_record('1', '1'), // 18-24 / MALE
            make_record('2', '2'), // 25-34 / FEMALE
            make_record('3', '3'), // 35-44 / OTHER
            make_record('6', '1'), // 65+   / MALE
            make_record('4', '2'), // 45-54 / FEMALE
            make_record('5', '1'), // 55-64 / MALE
        ]
    }

    // -----------------------------------------------------------------------
    // Basic end-to-end
    // -----------------------------------------------------------------------

    #[test]
    fn compute_weights_returns_one_weight_per_record() {
        let weights = compute_weights(&make_scheme(), Some(&make_rfl()), &make_data()).unwrap();
        assert_eq!(weights.len(), 6);
    }

    #[test]
    fn weights_sum_to_grand_total() {
        // Both tables' targets sum to 1.0 → grand total = 1.0.
        let weights = compute_weights(&make_scheme(), Some(&make_rfl()), &make_data()).unwrap();
        let sum: f64 = weights.iter().sum();
        assert!(
            (sum - 1.0).abs() < 1e-6,
            "weights sum = {sum}, expected 1.0"
        );
    }

    // -----------------------------------------------------------------------
    // Classification correctness
    // -----------------------------------------------------------------------

    #[test]
    fn classify_assigns_correct_category_indices() {
        let data = vec![
            make_record('1', '1'), // AGEGROUP cat 0 (18-44), GENDER cat 0 (MALE)
            make_record('6', '2'), // AGEGROUP cat 1 (65+),   GENDER cat 1 (FEMALE/OTHER)
            make_record('4', '3'), // AGEGROUP cat 2 (BALANCE), GENDER cat 1 (FEMALE/OTHER)
        ];

        let (survey, _) = classify(&make_scheme(), Some(&make_rfl()), &data).unwrap();

        // codes are stored [TABLE_601, TABLE_603] per record
        assert_eq!(survey.record_codes(0), &[0, 0]); // MALE, 18-44
        assert_eq!(survey.record_codes(1), &[1, 1]); // FEMALE/OTHER, 65+
        assert_eq!(survey.record_codes(2), &[1, 2]); // FEMALE/OTHER, BALANCE
    }

    // -----------------------------------------------------------------------
    // Error cases
    // -----------------------------------------------------------------------

    #[test]
    fn unclassified_record_returns_error() {
        // GENDER=' ' (blank) matches neither GENDER(1) nor GENDER(2,3).
        let data = vec![make_record('1', ' ')];
        let err = classify(&make_scheme(), Some(&make_rfl()), &data).unwrap_err();
        assert!(
            matches!(
                err,
                WeightError::Unclassified {
                    table_id: 601,
                    record: 0
                }
            ),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn multiple_matches_returns_error() {
        // Two overlapping categories: AGEGROUP(1,2,3) and AGEGROUP(1,4,5) both match code 1.
        let rfl = make_rfl();
        let scheme = WeightScheme {
            tables: vec![WeightTable {
                id: 603,
                label: None,
                categories: vec![
                    WeightCategory {
                        label: "YOUNG".to_string(),
                        condition: WeightCondition::Cfmc(
                            CfmcLogic::parse("AGEGROUP(1,2,3)").unwrap(),
                        ),
                        target: 0.5,
                    },
                    WeightCategory {
                        label: "OVERLAP".to_string(),
                        condition: WeightCondition::Cfmc(
                            CfmcLogic::parse("AGEGROUP(1,4,5)").unwrap(),
                        ),
                        target: 0.5,
                    },
                ],
            }],
            config: WeightConfig {
                raking: RakingConfig::default(),
                base_weight_field: None,
                base_weight_columns: None,
                target_tolerance: None,
            },
        };

        let data = vec![make_record('1', '1')];
        let err = classify(&scheme, Some(&rfl), &data).unwrap_err();
        assert!(
            matches!(
                err,
                WeightError::MultipleMatches {
                    table_id: 603,
                    record: 0,
                    ..
                }
            ),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn inconsistent_targets_return_target_mismatch_error() {
        // TABLE_601 targets sum to 1.0; TABLE_603 targets sum to 2.0 → inconsistent.
        let rfl = make_rfl();
        let scheme = WeightScheme {
            tables: vec![
                WeightTable {
                    id: 601,
                    label: None,
                    categories: vec![
                        WeightCategory {
                            label: "MALE".to_string(),
                            condition: WeightCondition::Cfmc(
                                CfmcLogic::parse("GENDER(1)").unwrap(),
                            ),
                            target: 0.48,
                        },
                        WeightCategory {
                            label: "FEMALE/OTHER".to_string(),
                            condition: WeightCondition::Cfmc(
                                CfmcLogic::parse("GENDER(2,3)").unwrap(),
                            ),
                            target: 0.52,
                        },
                    ],
                },
                WeightTable {
                    id: 603,
                    label: None,
                    categories: vec![
                        WeightCategory {
                            label: "18-44".to_string(),
                            condition: WeightCondition::Cfmc(
                                CfmcLogic::parse("AGEGROUP(1,2,3)").unwrap(),
                            ),
                            target: 1.0, // sum = 2.0, not 1.0
                        },
                        WeightCategory {
                            label: "45+".to_string(),
                            condition: WeightCondition::Cfmc(
                                CfmcLogic::parse("AGEGROUP(4,5,6,7)").unwrap(),
                            ),
                            target: 1.0,
                        },
                    ],
                },
            ],
            config: WeightConfig {
                raking: RakingConfig::default(),
                base_weight_field: None,
                base_weight_columns: None,
                target_tolerance: None,
            },
        };

        let data = vec![make_record('1', '1'), make_record('4', '2')];
        let err = classify(&scheme, Some(&rfl), &data).unwrap_err();
        assert!(
            matches!(err, WeightError::TargetMismatch { .. }),
            "unexpected error: {err}"
        );
    }

    // -----------------------------------------------------------------------
    // Base weight field
    // -----------------------------------------------------------------------

    #[test]
    fn base_weight_field_is_extracted_and_passed_to_survey() {
        let rfl = make_rfl_with_basewt();
        let scheme = WeightScheme {
            tables: vec![WeightTable {
                id: 601,
                label: None,
                categories: vec![
                    WeightCategory {
                        label: "MALE".to_string(),
                        condition: WeightCondition::Cfmc(CfmcLogic::parse("GENDER(1)").unwrap()),
                        target: 0.5,
                    },
                    WeightCategory {
                        label: "FEMALE/OTHER".to_string(),
                        condition: WeightCondition::Cfmc(CfmcLogic::parse("GENDER(2,3)").unwrap()),
                        target: 0.5,
                    },
                ],
            }],
            config: WeightConfig {
                raking: RakingConfig::default(),
                base_weight_field: Some("BASEWT".to_string()),
                base_weight_columns: None,
                target_tolerance: None,
            },
        };

        // "2.0  " occupies the 5-char BASEWT field (col 45, width 5).
        let data = vec![
            make_record_with_basewt('1', '1', "2.0  "),
            make_record_with_basewt('1', '2', "1.0  "),
            make_record_with_basewt('1', '1', "0.5  "),
        ];

        let (survey, _) = classify(&scheme, Some(&rfl), &data).unwrap();
        assert_eq!(survey.base_weight(0), 2.0);
        assert_eq!(survey.base_weight(1), 1.0);
        assert_eq!(survey.base_weight(2), 0.5);
    }

    #[test]
    fn base_weight_field_missing_from_rfl_returns_error() {
        let rfl = make_rfl(); // no BASEWT question
        let scheme = WeightScheme {
            tables: vec![WeightTable {
                id: 601,
                label: None,
                categories: vec![WeightCategory {
                    label: "MALE".to_string(),
                    condition: WeightCondition::Cfmc(CfmcLogic::parse("GENDER(1)").unwrap()),
                    target: 1.0,
                }],
            }],
            config: WeightConfig {
                raking: RakingConfig::default(),
                base_weight_field: Some("BASEWT".to_string()),
                base_weight_columns: None,
                target_tolerance: None,
            },
        };

        let data = vec![make_record('1', '1')];
        let err = classify(&scheme, Some(&rfl), &data).unwrap_err();
        assert!(
            matches!(&err, WeightError::BaseWeightField { field, .. } if field == "BASEWT"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn base_weight_field_non_numeric_returns_error() {
        let rfl = make_rfl_with_basewt();
        let scheme = WeightScheme {
            tables: vec![WeightTable {
                id: 601,
                label: None,
                categories: vec![
                    WeightCategory {
                        label: "MALE".to_string(),
                        condition: WeightCondition::Cfmc(CfmcLogic::parse("GENDER(1)").unwrap()),
                        target: 0.5,
                    },
                    WeightCategory {
                        label: "FEMALE/OTHER".to_string(),
                        condition: WeightCondition::Cfmc(CfmcLogic::parse("GENDER(2,3)").unwrap()),
                        target: 0.5,
                    },
                ],
            }],
            config: WeightConfig {
                raking: RakingConfig::default(),
                base_weight_field: Some("BASEWT".to_string()),
                base_weight_columns: None,
                target_tolerance: None,
            },
        };

        let data = vec![make_record_with_basewt('1', '1', "abc  ")];
        let err = classify(&scheme, Some(&rfl), &data).unwrap_err();
        assert!(
            matches!(&err, WeightError::BaseWeightField { record: 0, .. }),
            "unexpected error: {err}"
        );
    }

    // -----------------------------------------------------------------------
    // Base weight from columns (CWEIGHT/RETAIN)
    // -----------------------------------------------------------------------

    #[test]
    fn base_weight_columns_extracts_weights_from_raw_positions() {
        // Build records with a 5-char weight field at cols 45-49 (indices 44-48).
        // GENDER at col 43, AGEGROUP at col 41 (same as make_record).
        fn make_rec(agegroup: char, gender: char, weight: &str) -> String {
            let mut s = vec![b' '; 50];
            s[40] = agegroup as u8;
            s[42] = gender as u8;
            for (i, b) in weight.bytes().take(5).enumerate() {
                s[44 + i] = b;
            }
            String::from_utf8(s).unwrap()
        }

        let scheme = WeightScheme {
            tables: vec![WeightTable {
                id: 601,
                label: None,
                categories: vec![
                    WeightCategory {
                        label: "MALE".to_string(),
                        condition: WeightCondition::Uncle(
                            crate::weight::UncleExpr::parse("43-1").unwrap(),
                        ),
                        target: 0.5,
                    },
                    WeightCategory {
                        label: "FEMALE/OTHER".to_string(),
                        condition: WeightCondition::Uncle(
                            crate::weight::UncleExpr::parse("43-2:3").unwrap(),
                        ),
                        target: 0.5,
                    },
                ],
            }],
            config: WeightConfig {
                raking: RakingConfig::default(),
                base_weight_field: None,
                base_weight_columns: Some((45, 49)),
                target_tolerance: None,
            },
        };

        let data = vec![
            make_rec('1', '1', "2.0  "),
            make_rec('1', '2', "1.0  "),
            make_rec('1', '1', "0.5  "),
        ];

        let (survey, _) = classify(&scheme, None, &data).unwrap();
        assert_eq!(survey.base_weight(0), 2.0);
        assert_eq!(survey.base_weight(1), 1.0);
        assert_eq!(survey.base_weight(2), 0.5);
    }

    // -----------------------------------------------------------------------
    // Two-phase API
    // -----------------------------------------------------------------------

    #[test]
    fn two_phase_api_matches_compute_weights() {
        let rfl = make_rfl();
        let scheme = make_scheme();
        let data = make_data();

        let one_shot = compute_weights(&scheme, Some(&rfl), &data).unwrap();

        let (survey, targets) = classify(&scheme, Some(&rfl), &data).unwrap();
        let two_phase = rake_classified(&survey, &targets, &scheme.config.raking).unwrap();

        assert_eq!(one_shot.len(), two_phase.len());
        for (a, b) in one_shot.iter().zip(two_phase.iter()) {
            assert!((a - b).abs() < 1e-12, "weight mismatch: {a} vs {b}");
        }
    }

    // -----------------------------------------------------------------------
    // Display / error formatting
    // -----------------------------------------------------------------------

    #[test]
    fn weight_error_display_is_non_empty() {
        let errors: &[WeightError] = &[
            WeightError::ConditionEval {
                table_id: 601,
                row_label: "MALE".to_string(),
                record: 0,
                source: "question not found".to_string(),
            },
            WeightError::Unclassified {
                table_id: 601,
                record: 5,
            },
            WeightError::MultipleMatches {
                table_id: 603,
                record: 2,
                matched: vec!["18-44".to_string(), "OVERLAP".to_string()],
            },
            WeightError::BaseWeightField {
                field: "BASEWT".to_string(),
                record: 1,
                detail: "could not parse".to_string(),
            },
        ];
        for e in errors {
            assert!(!e.to_string().is_empty(), "empty Display for {e:?}");
        }
    }
}
