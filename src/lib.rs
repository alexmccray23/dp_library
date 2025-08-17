//! Survey data processing library
//!
//! This library provides functionality for parsing and processing survey data formats:
//! - RFL (Record Format Layout) parsing for survey question metadata
//! - CFMC logic expression parsing and evaluation for survey skip logic
//!
//! Originally ported from Perl survey processing tools.

pub mod cfmc;
pub mod crosstabs;
pub mod rfl;

// Re-export main types for convenience
pub use cfmc::{CfmcLogic, CfmcNode, CfmcOperator};
pub use crosstabs::{Banner, BannersTable, BannersTables, CrossTabsError, CrossTabsLogic};
pub use rfl::{QuestionType, RflFile, RflQuestion};
