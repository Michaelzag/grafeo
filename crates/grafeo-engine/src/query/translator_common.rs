//! Shared utilities for query language translators.
//!
//! Functions here are used by multiple translator modules (GQL, Cypher, etc.)
//! to avoid duplication of identical logic.

use std::sync::atomic::{AtomicU32, Ordering};

use super::plan::{AggregateFunction, BinaryOp, LogicalExpression};
use grafeo_common::utils::error::{Error, QueryError, QueryErrorKind, Result};

/// Returns true if the function name is a recognized aggregate function.
pub(crate) fn is_aggregate_function(name: &str) -> bool {
    matches!(
        name.to_uppercase().as_str(),
        "COUNT"
            | "SUM"
            | "AVG"
            | "MIN"
            | "MAX"
            | "COLLECT"
            | "STDEV"
            | "STDDEV"
            | "STDEVP"
            | "STDDEVP"
            | "PERCENTILE_DISC"
            | "PERCENTILEDISC"
            | "PERCENTILE_CONT"
            | "PERCENTILECONT"
    )
}

/// Converts a function name to an `AggregateFunction` enum variant.
pub(crate) fn to_aggregate_function(name: &str) -> Option<AggregateFunction> {
    match name.to_uppercase().as_str() {
        "COUNT" => Some(AggregateFunction::Count),
        "SUM" => Some(AggregateFunction::Sum),
        "AVG" => Some(AggregateFunction::Avg),
        "MIN" => Some(AggregateFunction::Min),
        "MAX" => Some(AggregateFunction::Max),
        "COLLECT" => Some(AggregateFunction::Collect),
        "STDEV" | "STDDEV" => Some(AggregateFunction::StdDev),
        "STDEVP" | "STDDEVP" => Some(AggregateFunction::StdDevPop),
        "PERCENTILE_DISC" | "PERCENTILEDISC" => Some(AggregateFunction::PercentileDisc),
        "PERCENTILE_CONT" | "PERCENTILECONT" => Some(AggregateFunction::PercentileCont),
        _ => None,
    }
}

/// Capitalizes the first character of a string.
///
/// Used by GraphQL translators to convert field names to type names.
pub(crate) fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

/// Generates unique variable names with an atomic counter.
///
/// Replaces the duplicated `var_counter: AtomicU32` + `next_var()` pattern
/// used across multiple translators.
pub(crate) struct VarGen {
    counter: AtomicU32,
}

impl VarGen {
    /// Creates a new variable generator starting from 0.
    pub fn new() -> Self {
        Self {
            counter: AtomicU32::new(0),
        }
    }

    /// Returns the next unique variable name (e.g., `_v0`, `_v1`, ...).
    pub fn next(&self) -> String {
        let n = self.counter.fetch_add(1, Ordering::Relaxed);
        format!("_v{n}")
    }

    /// Returns the current counter value without incrementing.
    pub fn current(&self) -> u32 {
        self.counter.load(Ordering::Relaxed)
    }
}

/// Combines a non-empty vector of predicates into a single AND expression.
///
/// Returns an error if the input is empty. Used by `build_property_predicate`
/// in multiple translators.
pub(crate) fn combine_with_and(predicates: Vec<LogicalExpression>) -> Result<LogicalExpression> {
    predicates
        .into_iter()
        .reduce(|acc, pred| LogicalExpression::Binary {
            left: Box::new(acc),
            op: BinaryOp::And,
            right: Box::new(pred),
        })
        .ok_or_else(|| {
            Error::Query(QueryError::new(
                QueryErrorKind::Semantic,
                "Empty property predicate",
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- capitalize_first ---

    #[test]
    fn capitalize_first_empty() {
        assert_eq!(capitalize_first(""), "");
    }

    #[test]
    fn capitalize_first_single_char() {
        assert_eq!(capitalize_first("a"), "A");
    }

    #[test]
    fn capitalize_first_already_upper() {
        assert_eq!(capitalize_first("Hello"), "Hello");
    }

    #[test]
    fn capitalize_first_lower() {
        assert_eq!(capitalize_first("person"), "Person");
    }

    // --- VarGen ---

    #[test]
    fn var_gen_starts_at_zero() {
        let vg = VarGen::new();
        assert_eq!(vg.current(), 0);
    }

    #[test]
    fn var_gen_increments() {
        let vg = VarGen::new();
        assert_eq!(vg.next(), "_v0");
        assert_eq!(vg.next(), "_v1");
        assert_eq!(vg.current(), 2);
    }

    // --- combine_with_and ---

    #[test]
    fn combine_with_and_empty_returns_error() {
        let result = combine_with_and(vec![]);
        assert!(result.is_err());
    }

    #[test]
    fn combine_with_and_single_predicate() {
        let pred = LogicalExpression::Property {
            variable: "n".to_string(),
            property: "name".to_string(),
        };
        let result = combine_with_and(vec![pred.clone()]).unwrap();
        assert!(matches!(result, LogicalExpression::Property { .. }));
    }

    #[test]
    fn combine_with_and_two_predicates() {
        let p1 = LogicalExpression::Property {
            variable: "n".to_string(),
            property: "a".to_string(),
        };
        let p2 = LogicalExpression::Property {
            variable: "n".to_string(),
            property: "b".to_string(),
        };
        let result = combine_with_and(vec![p1, p2]).unwrap();
        assert!(matches!(
            result,
            LogicalExpression::Binary {
                op: BinaryOp::And,
                ..
            }
        ));
    }
}
