//! SHACL validation report.
//!
//! Implements the W3C SHACL Validation Report vocabulary, providing
//! structured results that can be materialized as RDF triples.

use std::fmt;

use crate::graph::rdf::Term;

use super::shape::{PropertyPath, RDF, SH, Severity};

/// A SHACL validation report.
#[derive(Debug, Clone)]
pub struct ValidationReport {
    /// Whether the data conforms to all shapes (`true` = no violations).
    pub conforms: bool,
    /// Individual validation results.
    pub results: Vec<ValidationResult>,
}

impl ValidationReport {
    /// Creates a conforming report (no violations).
    #[must_use]
    pub fn conforming() -> Self {
        Self {
            conforms: true,
            results: Vec::new(),
        }
    }

    /// Creates a report from a set of results. Conforms if no violations.
    #[must_use]
    pub fn from_results(results: Vec<ValidationResult>) -> Self {
        let conforms = results.iter().all(|r| r.severity != Severity::Violation);
        Self { conforms, results }
    }
}

/// A single SHACL validation result.
#[derive(Debug, Clone)]
pub struct ValidationResult {
    /// The focus node that was validated.
    pub focus_node: Term,
    /// The constraint component that produced this result (IRI).
    pub source_constraint_component: String,
    /// The shape that produced this result.
    pub source_shape: Term,
    /// The value node that caused the violation (if applicable).
    pub value: Option<Term>,
    /// The property path (for property shape results).
    pub result_path: Option<PropertyPath>,
    /// The severity level.
    pub severity: Severity,
    /// A human-readable message.
    pub message: Option<String>,
}

impl fmt::Display for ValidationReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let violations = self
            .results
            .iter()
            .filter(|r| r.severity == Severity::Violation)
            .count();
        let warnings = self
            .results
            .iter()
            .filter(|r| r.severity == Severity::Warning)
            .count();

        if self.conforms {
            write!(f, "Validation Report: PASSED")?;
        } else {
            write!(f, "Validation Report: FAILED")?;
        }
        if violations > 0 || warnings > 0 {
            write!(f, " ({violations} violation(s), {warnings} warning(s))")?;
        }
        writeln!(f)?;

        for result in &self.results {
            let severity = match result.severity {
                Severity::Violation => "Violation",
                Severity::Warning => "Warning",
                Severity::Info => "Info",
            };
            write!(f, "  [{severity}] {}", result.focus_node)?;
            if let Some(ref msg) = result.message {
                write!(f, " - {msg}")?;
            }
            writeln!(f)?;
        }

        Ok(())
    }
}

/// Materializes the validation report as RDF triples.
///
/// Produces triples following the W3C SHACL Validation Report vocabulary
/// (section 3.6 of the spec).
impl ValidationReport {
    /// Converts the report to a set of RDF triples.
    #[must_use]
    pub fn to_triples(&self) -> Vec<crate::graph::rdf::Triple> {
        use crate::graph::rdf::Triple;

        let mut triples = Vec::new();
        let report_node = Term::blank("report");
        let rdf_type = Term::iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#type");

        // Report type
        triples.push(Triple::new(
            report_node.clone(),
            rdf_type.clone(),
            Term::iri(SH::VALIDATION_REPORT),
        ));

        // sh:conforms
        triples.push(Triple::new(
            report_node.clone(),
            Term::iri(SH::CONFORMS),
            Term::typed_literal(
                if self.conforms { "true" } else { "false" },
                "http://www.w3.org/2001/XMLSchema#boolean",
            ),
        ));

        // Each result
        for (i, result) in self.results.iter().enumerate() {
            let result_node = Term::blank(format!("result_{i}"));
            triples.push(Triple::new(
                report_node.clone(),
                Term::iri(SH::RESULT),
                result_node.clone(),
            ));
            triples.push(Triple::new(
                result_node.clone(),
                rdf_type.clone(),
                Term::iri(SH::VALIDATION_RESULT),
            ));

            // Focus node
            triples.push(Triple::new(
                result_node.clone(),
                Term::iri(SH::FOCUS_NODE),
                result.focus_node.clone(),
            ));

            // Source constraint component
            triples.push(Triple::new(
                result_node.clone(),
                Term::iri(SH::SOURCE_CONSTRAINT_COMPONENT),
                Term::iri(&*result.source_constraint_component),
            ));

            // Source shape
            triples.push(Triple::new(
                result_node.clone(),
                Term::iri(SH::SOURCE_SHAPE),
                result.source_shape.clone(),
            ));

            // Value (optional)
            if let Some(ref value) = result.value {
                triples.push(Triple::new(
                    result_node.clone(),
                    Term::iri(SH::VALUE),
                    value.clone(),
                ));
            }

            // Result path (optional)
            if let Some(ref path) = result.result_path {
                let path_node = serialize_path(path, i, &mut triples);
                triples.push(Triple::new(
                    result_node.clone(),
                    Term::iri(SH::RESULT_PATH),
                    path_node,
                ));
            }

            // Severity
            let severity_iri = match result.severity {
                Severity::Violation => SH::SEVERITY_VIOLATION,
                Severity::Warning => SH::SEVERITY_WARNING,
                Severity::Info => SH::SEVERITY_INFO,
            };
            triples.push(Triple::new(
                result_node.clone(),
                Term::iri(SH::RESULT_SEVERITY),
                Term::iri(severity_iri),
            ));

            // Message (optional)
            if let Some(ref msg) = result.message {
                triples.push(Triple::new(
                    result_node,
                    Term::iri(SH::RESULT_MESSAGE),
                    Term::literal(msg.as_str()),
                ));
            }
        }

        triples
    }
}

/// Serializes a [`PropertyPath`] as RDF triples and returns the term representing it.
///
/// Simple predicate paths return the IRI directly. Complex paths produce blank-node
/// structures following the SHACL path vocabulary.
fn serialize_path(
    path: &PropertyPath,
    result_idx: usize,
    triples: &mut Vec<crate::graph::rdf::Triple>,
) -> Term {
    use crate::graph::rdf::Triple;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    match path {
        PropertyPath::Predicate(iri) => iri.clone(),
        PropertyPath::Inverse(inner) => {
            let bnode = Term::blank(format!(
                "path_{result_idx}_{}",
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            let inner_term = serialize_path(inner, result_idx, triples);
            triples.push(Triple::new(
                bnode.clone(),
                Term::iri(SH::INVERSE_PATH),
                inner_term,
            ));
            bnode
        }
        PropertyPath::ZeroOrMore(inner) => {
            let bnode = Term::blank(format!(
                "path_{result_idx}_{}",
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            let inner_term = serialize_path(inner, result_idx, triples);
            triples.push(Triple::new(
                bnode.clone(),
                Term::iri(SH::ZERO_OR_MORE_PATH),
                inner_term,
            ));
            bnode
        }
        PropertyPath::OneOrMore(inner) => {
            let bnode = Term::blank(format!(
                "path_{result_idx}_{}",
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            let inner_term = serialize_path(inner, result_idx, triples);
            triples.push(Triple::new(
                bnode.clone(),
                Term::iri(SH::ONE_OR_MORE_PATH),
                inner_term,
            ));
            bnode
        }
        PropertyPath::ZeroOrOne(inner) => {
            let bnode = Term::blank(format!(
                "path_{result_idx}_{}",
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            let inner_term = serialize_path(inner, result_idx, triples);
            triples.push(Triple::new(
                bnode.clone(),
                Term::iri(SH::ZERO_OR_ONE_PATH),
                inner_term,
            ));
            bnode
        }
        PropertyPath::Sequence(paths) => serialize_rdf_list(paths, result_idx, triples),
        PropertyPath::Alternative(paths) => {
            let bnode = Term::blank(format!(
                "path_{result_idx}_{}",
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            let list_head = serialize_rdf_list(paths, result_idx, triples);
            triples.push(Triple::new(
                bnode.clone(),
                Term::iri(SH::ALTERNATIVE_PATH),
                list_head,
            ));
            bnode
        }
    }
}

/// Serializes a list of paths as an RDF collection (rdf:first/rdf:rest chain).
fn serialize_rdf_list(
    paths: &[PropertyPath],
    result_idx: usize,
    triples: &mut Vec<crate::graph::rdf::Triple>,
) -> Term {
    use crate::graph::rdf::Triple;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    if paths.is_empty() {
        return Term::iri(RDF::NIL);
    }

    let mut head = None;
    let mut prev: Option<Term> = None;

    for path in paths {
        let node = Term::blank(format!(
            "list_{result_idx}_{}",
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let item = serialize_path(path, result_idx, triples);
        triples.push(Triple::new(node.clone(), Term::iri(RDF::FIRST), item));

        if let Some(prev_node) = prev {
            triples.push(Triple::new(prev_node, Term::iri(RDF::REST), node.clone()));
        }
        if head.is_none() {
            head = Some(node.clone());
        }
        prev = Some(node);
    }

    // Close the list
    if let Some(last) = prev {
        triples.push(Triple::new(last, Term::iri(RDF::REST), Term::iri(RDF::NIL)));
    }

    head.unwrap_or_else(|| Term::iri(RDF::NIL))
}
