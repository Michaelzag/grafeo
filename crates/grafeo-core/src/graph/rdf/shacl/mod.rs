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

pub use constraint::evaluate_constraint;
pub use parser::parse_shapes;
pub use path::evaluate_path;
pub use report::{ValidationReport, ValidationResult};
pub use shape::{
    Constraint, NodeKindValue, NodeShape, PropertyPath, PropertyShape, SH, Severity, ShaclError,
    Shape, Target,
};
pub use target::resolve_targets;
