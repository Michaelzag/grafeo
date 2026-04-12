//! SHACL (Shapes Constraint Language) validation for RDF graphs.
//!
//! This module implements the W3C SHACL specification for validating RDF data
//! against shape definitions. It supports both SHACL Core constraints (evaluated
//! purely against the RDF store) and SHACL-SPARQL constraints (evaluated via
//! an optional SPARQL executor callback).
//!
//! # Architecture
//!
//! - **Shape model** (`shape.rs`): data types for shapes, targets, paths, constraints
//! - **Parser** (`parser.rs`): reads shape definitions from an RDF store
//! - **Target resolution** (`target.rs`): finds focus nodes for each shape
//! - **Path evaluation** (`path.rs`): evaluates SHACL property paths
//! - **Constraint evaluation** (`constraint.rs`): checks constraints against value nodes
//! - **Report** (`report.rs`): validation results in W3C format

pub mod constraint;
mod parser;
pub mod path;
pub mod report;
pub mod shape;
mod target;

use std::collections::{HashMap, HashSet};

use crate::graph::rdf::{RdfStore, Term};

pub use constraint::evaluate_constraint;
pub use parser::parse_shapes;
pub use path::evaluate_path;
pub use report::{ValidationReport, ValidationResult};
pub use shape::{
    Constraint, NodeKindValue, NodeShape, PropertyPath, PropertyShape, SH, Severity, ShaclError,
    Shape, Target,
};
pub use target::resolve_targets;

// =========================================================================
// SPARQL executor trait (for core/engine decoupling)
// =========================================================================

/// Trait for executing SPARQL queries from SHACL-SPARQL constraints.
///
/// Implemented by the engine layer (`SessionSparqlExecutor`) to provide
/// query execution without making grafeo-core depend on grafeo-engine.
pub trait SparqlExecutor {
    /// Executes a SPARQL SELECT query with `$this` bound to the focus node.
    ///
    /// Returns result rows as maps of variable name to term value.
    ///
    /// # Errors
    ///
    /// Returns `ShaclError::SparqlError` if execution fails.
    fn execute(
        &self,
        query: &str,
        this_binding: &Term,
    ) -> Result<Vec<HashMap<String, Term>>, ShaclError>;
}

// =========================================================================
// Top-level validation orchestration
// =========================================================================

/// Validates RDF data against SHACL shapes.
///
/// Parses shapes from the shapes graph, resolves targets in the data graph,
/// evaluates all constraints, and returns a validation report.
///
/// # Arguments
///
/// * `data_graph` - The RDF store containing data to validate
/// * `shapes_graph` - The RDF store containing SHACL shape definitions
/// * `sparql_executor` - Optional executor for SHACL-SPARQL constraints
///
/// # Errors
///
/// Returns `ShaclError` if shape parsing fails or a constraint cannot be evaluated.
pub fn validate(
    data_graph: &RdfStore,
    shapes_graph: &RdfStore,
    sparql_executor: Option<&dyn SparqlExecutor>,
) -> Result<ValidationReport, ShaclError> {
    let shapes = parse_shapes(shapes_graph)?;
    let mut all_results = Vec::new();

    for shape in &shapes {
        if shape.is_deactivated() {
            continue;
        }

        let focus_nodes = resolve_targets(shape, data_graph);
        for focus_node in &focus_nodes {
            let results = validate_shape(shape, focus_node, data_graph, &shapes, sparql_executor)?;
            all_results.extend(results);
        }
    }

    Ok(ValidationReport::from_results(all_results))
}

/// Validates a single shape against a focus node.
fn validate_shape(
    shape: &Shape,
    focus_node: &Term,
    data_graph: &RdfStore,
    all_shapes: &[Shape],
    sparql_executor: Option<&dyn SparqlExecutor>,
) -> Result<Vec<ValidationResult>, ShaclError> {
    let mut visited = HashSet::new();
    let mut results = Vec::new();

    match shape {
        Shape::Node(ns) => {
            // Evaluate node-level constraints with focus node as the single value node
            let mut ctx = constraint::EvalContext {
                focus_node,
                shape,
                path: None,
                data_graph,
                all_shapes,
                visited: &mut visited,
            };
            for c in &ns.constraints {
                let value_nodes = vec![focus_node.clone()];
                results.extend(evaluate_constraint(c, &value_nodes, &mut ctx));
            }

            // Evaluate SPARQL constraints on the node shape
            for c in &ns.constraints {
                if let Constraint::Sparql(sc) = c {
                    results.extend(evaluate_sparql_constraint(
                        sc,
                        focus_node,
                        shape,
                        None,
                        sparql_executor,
                    )?);
                }
            }

            // Evaluate nested property shapes
            for ps in &ns.property_shapes {
                if ps.deactivated {
                    continue;
                }
                let path_values = evaluate_path(&ps.path, focus_node, data_graph);
                let ps_shape = Shape::Property(ps.clone());
                let mut ps_ctx = constraint::EvalContext {
                    focus_node,
                    shape: &ps_shape,
                    path: Some(&ps.path),
                    data_graph,
                    all_shapes,
                    visited: &mut visited,
                };
                for c in &ps.constraints {
                    if let Constraint::Sparql(sc) = c {
                        results.extend(evaluate_sparql_constraint(
                            sc,
                            focus_node,
                            &ps_shape,
                            Some(&ps.path),
                            sparql_executor,
                        )?);
                    } else {
                        results.extend(evaluate_constraint(c, &path_values, &mut ps_ctx));
                    }
                }
            }
        }
        Shape::Property(ps) => {
            let path_values = evaluate_path(&ps.path, focus_node, data_graph);
            let mut ctx = constraint::EvalContext {
                focus_node,
                shape,
                path: Some(&ps.path),
                data_graph,
                all_shapes,
                visited: &mut visited,
            };
            for c in &ps.constraints {
                if let Constraint::Sparql(sc) = c {
                    results.extend(evaluate_sparql_constraint(
                        sc,
                        focus_node,
                        shape,
                        Some(&ps.path),
                        sparql_executor,
                    )?);
                } else {
                    results.extend(evaluate_constraint(c, &path_values, &mut ctx));
                }
            }
        }
    }

    Ok(results)
}

/// Evaluates a SHACL-SPARQL constraint using the optional executor.
fn evaluate_sparql_constraint(
    sc: &shape::SparqlConstraint,
    focus_node: &Term,
    shape: &Shape,
    result_path: Option<&PropertyPath>,
    sparql_executor: Option<&dyn SparqlExecutor>,
) -> Result<Vec<ValidationResult>, ShaclError> {
    if sc.deactivated {
        return Ok(Vec::new());
    }

    let Some(executor) = sparql_executor else {
        // No executor provided: skip SPARQL constraints silently
        return Ok(Vec::new());
    };

    // Build the full query with prefix declarations
    let mut query = String::new();
    for decl in &sc.prefixes {
        use std::fmt::Write;
        let _ = writeln!(query, "PREFIX {}: <{}>", decl.prefix, decl.namespace);
    }
    query.push_str(&sc.select);

    // Execute the query, propagating errors instead of swallowing them
    let rows = executor.execute(&query, focus_node)?;

    // Each result row is a violation
    let mut results = Vec::new();
    for row in &rows {
        let value = row.get("value").cloned();
        let message = row
            .get("message")
            .and_then(|t| match t {
                Term::Literal(lit) => Some(lit.value().to_string()),
                _ => None,
            })
            .or_else(|| sc.message.clone());

        results.push(ValidationResult {
            focus_node: focus_node.clone(),
            source_constraint_component: format!("{}SPARQLConstraintComponent", SH::NS),
            source_shape: shape.id().clone(),
            value,
            result_path: result_path.cloned(),
            severity: shape.severity(),
            message,
        });
    }

    Ok(results)
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::rdf::{RdfStore, Triple};

    #[test]
    fn validate_conforming_data() {
        let data = RdfStore::new();
        let rdf_type = Term::iri(shape::RDF::TYPE);
        data.insert(Triple::new(
            Term::iri("http://ex.org/alix"),
            rdf_type,
            Term::iri("http://ex.org/Person"),
        ));
        data.insert(Triple::new(
            Term::iri("http://ex.org/alix"),
            Term::iri("http://ex.org/name"),
            Term::literal("Alix"),
        ));

        let shapes = RdfStore::new();
        let shape_id = Term::iri("http://ex.org/PersonShape");
        shapes.insert(Triple::new(
            shape_id.clone(),
            Term::iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#type"),
            Term::iri(SH::NODE_SHAPE),
        ));
        shapes.insert(Triple::new(
            shape_id.clone(),
            Term::iri(SH::TARGET_CLASS),
            Term::iri("http://ex.org/Person"),
        ));
        let prop = Term::blank("p");
        shapes.insert(Triple::new(shape_id, Term::iri(SH::PROPERTY), prop.clone()));
        shapes.insert(Triple::new(
            prop.clone(),
            Term::iri(SH::PATH),
            Term::iri("http://ex.org/name"),
        ));
        shapes.insert(Triple::new(
            prop,
            Term::iri(SH::MIN_COUNT),
            Term::typed_literal("1", "http://www.w3.org/2001/XMLSchema#integer"),
        ));

        let report = validate(&data, &shapes, None).unwrap();
        assert!(report.conforms, "Data should conform: {report}");
    }

    #[test]
    fn validate_with_violations() {
        let data = RdfStore::new();
        let rdf_type = Term::iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#type");
        data.insert(Triple::new(
            Term::iri("http://ex.org/alix"),
            rdf_type,
            Term::iri("http://ex.org/Person"),
        ));
        // No name property: violates minCount 1

        let shapes = RdfStore::new();
        let shape_id = Term::iri("http://ex.org/PersonShape");
        shapes.insert(Triple::new(
            shape_id.clone(),
            Term::iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#type"),
            Term::iri(SH::NODE_SHAPE),
        ));
        shapes.insert(Triple::new(
            shape_id.clone(),
            Term::iri(SH::TARGET_CLASS),
            Term::iri("http://ex.org/Person"),
        ));
        let prop = Term::blank("p");
        shapes.insert(Triple::new(shape_id, Term::iri(SH::PROPERTY), prop.clone()));
        shapes.insert(Triple::new(
            prop.clone(),
            Term::iri(SH::PATH),
            Term::iri("http://ex.org/name"),
        ));
        shapes.insert(Triple::new(
            prop,
            Term::iri(SH::MIN_COUNT),
            Term::typed_literal("1", "http://www.w3.org/2001/XMLSchema#integer"),
        ));

        let report = validate(&data, &shapes, None).unwrap();
        assert!(!report.conforms);
        assert_eq!(report.results.len(), 1);
    }

    #[test]
    fn empty_shapes_conforms() {
        let data = RdfStore::new();
        let shapes = RdfStore::new();
        let report = validate(&data, &shapes, None).unwrap();
        assert!(report.conforms);
        assert!(report.results.is_empty());
    }

    #[test]
    fn deactivated_shape_skipped() {
        let data = RdfStore::new();
        let rdf_type = Term::iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#type");
        data.insert(Triple::new(
            Term::iri("http://ex.org/alix"),
            rdf_type,
            Term::iri("http://ex.org/Person"),
        ));

        let shapes = RdfStore::new();
        let shape_id = Term::iri("http://ex.org/PersonShape");
        shapes.insert(Triple::new(
            shape_id.clone(),
            Term::iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#type"),
            Term::iri(SH::NODE_SHAPE),
        ));
        shapes.insert(Triple::new(
            shape_id.clone(),
            Term::iri(SH::TARGET_CLASS),
            Term::iri("http://ex.org/Person"),
        ));
        shapes.insert(Triple::new(
            shape_id.clone(),
            Term::iri(SH::DEACTIVATED),
            Term::typed_literal("true", "http://www.w3.org/2001/XMLSchema#boolean"),
        ));
        let prop = Term::blank("p");
        shapes.insert(Triple::new(shape_id, Term::iri(SH::PROPERTY), prop.clone()));
        shapes.insert(Triple::new(
            prop.clone(),
            Term::iri(SH::PATH),
            Term::iri("http://ex.org/name"),
        ));
        shapes.insert(Triple::new(
            prop,
            Term::iri(SH::MIN_COUNT),
            Term::typed_literal("1", "http://www.w3.org/2001/XMLSchema#integer"),
        ));

        let report = validate(&data, &shapes, None).unwrap();
        assert!(report.conforms, "Deactivated shape should be skipped");
    }

    #[test]
    fn report_to_triples() {
        let data = RdfStore::new();
        let rdf_type = Term::iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#type");
        data.insert(Triple::new(
            Term::iri("http://ex.org/alix"),
            rdf_type,
            Term::iri("http://ex.org/Person"),
        ));

        let shapes = RdfStore::new();
        let shape_id = Term::iri("http://ex.org/S");
        shapes.insert(Triple::new(
            shape_id.clone(),
            Term::iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#type"),
            Term::iri(SH::NODE_SHAPE),
        ));
        shapes.insert(Triple::new(
            shape_id.clone(),
            Term::iri(SH::TARGET_CLASS),
            Term::iri("http://ex.org/Person"),
        ));
        let prop = Term::blank("p");
        shapes.insert(Triple::new(shape_id, Term::iri(SH::PROPERTY), prop.clone()));
        shapes.insert(Triple::new(
            prop.clone(),
            Term::iri(SH::PATH),
            Term::iri("http://ex.org/name"),
        ));
        shapes.insert(Triple::new(
            prop,
            Term::iri(SH::MIN_COUNT),
            Term::typed_literal("1", "http://www.w3.org/2001/XMLSchema#integer"),
        ));

        let report = validate(&data, &shapes, None).unwrap();
        let triples = report.to_triples();
        // Should have: report type, conforms, result link, result type, focus, component, shape, severity, message
        assert!(
            triples.len() >= 5,
            "Expected at least 5 triples, got {}",
            triples.len()
        );
    }

    #[test]
    fn display_format() {
        let report = ValidationReport::from_results(vec![ValidationResult {
            focus_node: Term::iri("http://ex.org/alix"),
            source_constraint_component: format!("{}MinCountConstraintComponent", SH::NS),
            source_shape: Term::iri("http://ex.org/S"),
            value: None,
            result_path: Some(PropertyPath::Predicate(Term::iri("http://ex.org/name"))),
            severity: Severity::Violation,
            message: Some("Expected at least 1 value(s), got 0".to_string()),
        }]);
        let text = format!("{report}");
        assert!(text.contains("FAILED"));
        assert!(text.contains("Violation"));
    }
}
