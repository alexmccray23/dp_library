//! Survey data processing library
//!
//! This library provides functionality for parsing and processing survey data formats:
//! - RFL (Record Format Layout) parsing for survey question metadata
//! - CFMC logic expression parsing and evaluation for survey skip logic
//! 
//! Originally ported from Perl survey processing tools.

pub mod rfl;
pub mod cfmc;

// Re-export main types for convenience
pub use rfl::{RflFile, RflQuestion, QuestionType};
pub use cfmc::{CfmcLogic, CfmcNode, CfmcOperator};