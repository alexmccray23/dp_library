pub mod ipf;
pub mod parse_e;
pub mod uncle;

/// Convert a 0-based character index to a byte offset within `s`.
///
/// If `char_idx` exceeds the number of characters, returns `s.len()`.
#[must_use]
pub fn char_to_byte(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map_or(s.len(), |(byte_offset, _)| byte_offset)
}

/// Extract a substring using 1-based inclusive character columns.
///
/// Returns the slice `s[col_start..=col_end]` measured in characters.
/// If the string is shorter than `col_end`, returns as many characters as
/// available (or an empty string if shorter than `col_start`).
#[must_use]
pub fn char_substr(s: &str, col_start: usize, col_end: usize) -> &str {
    let byte_start = char_to_byte(s, col_start - 1);
    let byte_end = char_to_byte(s, col_end);
    if byte_start >= s.len() {
        ""
    } else if byte_end > s.len() {
        &s[byte_start..]
    } else {
        &s[byte_start..byte_end]
    }
}

pub use ipf::{
    MultiPassResult, WeightCategory, WeightCondition, WeightConfig, WeightError, WeightScheme,
    WeightTable, classify, compute_weights, compute_weights_multi_pass, rake_classified,
    rake_classified_full,
};
pub use parse_e::{ColumnAssignment, MoveDirective, ParseEError, ParsedWeightSpec, QualifiedWeightPass, WeightDirective, parse_e_file};
pub use uncle::{
    CodeVal, LiteralMatch, ParseError, RQual, RValue, Relation, PQual, UncleExpr,
};
