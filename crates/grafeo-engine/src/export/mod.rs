//! Graph export serializers (GEXF, GraphML).
//!
//! Provides streaming serializers that write graph data directly to a [`std::io::Write`] sink.
//! No external XML library is needed: both formats are simple enough for `write!()` macros
//! with proper escaping.

pub mod gexf;
pub mod graphml;

use std::collections::BTreeMap;
use std::io;

use grafeo_common::PropertyKey;
use grafeo_common::types::Value;
use grafeo_core::graph::lpg::{Edge, Node};

/// Errors from graph export operations.
#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    /// I/O error while writing output.
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
}

/// Escapes XML special characters in text content and attribute values.
#[must_use]
pub fn escape_xml(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => result.push_str("&amp;"),
            '<' => result.push_str("&lt;"),
            '>' => result.push_str("&gt;"),
            '"' => result.push_str("&quot;"),
            '\'' => result.push_str("&apos;"),
            _ => result.push(ch),
        }
    }
    result
}

/// Maps a grafeo [`Value`] to a GEXF attribute type string.
#[must_use]
pub fn value_to_gexf_type(value: &Value) -> &'static str {
    match value {
        Value::Int64(_) => "integer",
        Value::Float64(_) => "float",
        Value::Bool(_) => "boolean",
        Value::String(_) => "string",
        Value::Date(_) => "date",
        _ => "string",
    }
}

/// Maps a grafeo [`Value`] to a GraphML attribute type string.
#[must_use]
pub fn value_to_graphml_type(value: &Value) -> &'static str {
    match value {
        Value::Int64(_) => "long",
        Value::Float64(_) => "double",
        Value::Bool(_) => "boolean",
        Value::String(_) => "string",
        _ => "string",
    }
}

/// Converts a [`Value`] to an XML-safe string representation.
///
/// Returns `None` for `Value::Null` (callers should omit the element).
#[must_use]
pub fn value_to_xml_string(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::Bool(b) => Some(b.to_string()),
        Value::Int64(i) => Some(i.to_string()),
        Value::Float64(f) => Some(f.to_string()),
        Value::String(s) => Some(escape_xml(s.as_str())),
        Value::Date(d) => Some(d.to_string()),
        Value::Time(t) => Some(t.to_string()),
        Value::Timestamp(ts) => Some(ts.to_string()),
        Value::Duration(d) => Some(d.to_string()),
        Value::ZonedDatetime(zdt) => Some(zdt.to_string()),
        Value::Bytes(b) => {
            // Hex-encode binary data
            use std::fmt::Write;
            let hex = b.iter().fold(String::new(), |mut acc, byte| {
                let _ = write!(acc, "{byte:02x}");
                acc
            });
            Some(hex)
        }
        Value::Vector(v) => {
            let parts: Vec<String> = v.iter().map(|f| f.to_string()).collect();
            Some(parts.join(","))
        }
        Value::List(items) => {
            let parts: Vec<String> = items.iter().filter_map(value_to_xml_string).collect();
            Some(parts.join(","))
        }
        Value::Map(m) => {
            // Serialize as key=value pairs
            let parts: Vec<String> = m
                .iter()
                .map(|(k, v)| {
                    let val_str = value_to_xml_string(v).unwrap_or_default();
                    format!("{}={}", escape_xml(k.as_str()), val_str)
                })
                .collect();
            Some(parts.join(";"))
        }
        Value::Path { .. } | Value::GCounter(_) | Value::OnCounter { .. } => {
            Some(escape_xml(&value.to_string()))
        }
    }
}

/// Discovered property schema: maps property key to (attribute ID, GEXF/GraphML type).
pub(crate) type PropertySchema = BTreeMap<PropertyKey, (usize, &'static str)>;

/// Discovers the property schema for nodes by scanning all property keys and their types.
pub(crate) fn discover_node_schema<F>(nodes: &[Node], type_fn: F) -> PropertySchema
where
    F: Fn(&Value) -> &'static str,
{
    let mut schema: BTreeMap<PropertyKey, Option<&'static str>> = BTreeMap::new();
    for node in nodes {
        for (key, value) in node.properties.iter() {
            schema
                .entry(key.clone())
                .and_modify(|existing| {
                    if existing.is_none() && !value.is_null() {
                        *existing = Some(type_fn(value));
                    }
                })
                .or_insert_with(|| {
                    if value.is_null() {
                        None
                    } else {
                        Some(type_fn(value))
                    }
                });
        }
    }
    schema
        .into_iter()
        .enumerate()
        .map(|(idx, (key, type_str))| (key, (idx, type_str.unwrap_or("string"))))
        .collect()
}

/// Discovers the property schema for edges by scanning all property keys and their types.
pub(crate) fn discover_edge_schema<F>(edges: &[Edge], type_fn: F) -> PropertySchema
where
    F: Fn(&Value) -> &'static str,
{
    let mut schema: BTreeMap<PropertyKey, Option<&'static str>> = BTreeMap::new();
    for edge in edges {
        for (key, value) in edge.properties.iter() {
            schema
                .entry(key.clone())
                .and_modify(|existing| {
                    if existing.is_none() && !value.is_null() {
                        *existing = Some(type_fn(value));
                    }
                })
                .or_insert_with(|| {
                    if value.is_null() {
                        None
                    } else {
                        Some(type_fn(value))
                    }
                });
        }
    }
    schema
        .into_iter()
        .enumerate()
        .map(|(idx, (key, type_str))| (key, (idx, type_str.unwrap_or("string"))))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_xml_basic() {
        assert_eq!(escape_xml("hello"), "hello");
        assert_eq!(escape_xml("a & b"), "a &amp; b");
        assert_eq!(escape_xml("<tag>"), "&lt;tag&gt;");
        assert_eq!(escape_xml("she said \"hi\""), "she said &quot;hi&quot;");
        assert_eq!(escape_xml("it's"), "it&apos;s");
    }

    #[test]
    fn test_escape_xml_combined() {
        assert_eq!(
            escape_xml("<a href=\"x&y\">"),
            "&lt;a href=&quot;x&amp;y&quot;&gt;"
        );
    }

    #[test]
    fn test_value_to_xml_string_null() {
        assert!(value_to_xml_string(&Value::Null).is_none());
    }

    #[test]
    fn test_value_to_xml_string_primitives() {
        assert_eq!(value_to_xml_string(&Value::Bool(true)).unwrap(), "true");
        assert_eq!(value_to_xml_string(&Value::Int64(42)).unwrap(), "42");
        assert_eq!(
            value_to_xml_string(&Value::Float64(3.125)).unwrap(),
            "3.125"
        );
        assert_eq!(
            value_to_xml_string(&Value::String("Alix & Gus".into())).unwrap(),
            "Alix &amp; Gus"
        );
    }

    #[test]
    fn test_value_to_xml_string_vector() {
        let v = Value::Vector(std::sync::Arc::from(vec![1.0f32, 2.0, 3.0].as_slice()));
        assert_eq!(value_to_xml_string(&v).unwrap(), "1,2,3");
    }

    #[test]
    fn test_gexf_type_mapping() {
        assert_eq!(value_to_gexf_type(&Value::Int64(0)), "integer");
        assert_eq!(value_to_gexf_type(&Value::Float64(0.0)), "float");
        assert_eq!(value_to_gexf_type(&Value::Bool(true)), "boolean");
        assert_eq!(value_to_gexf_type(&Value::String("".into())), "string");
    }

    #[test]
    fn test_graphml_type_mapping() {
        assert_eq!(value_to_graphml_type(&Value::Int64(0)), "long");
        assert_eq!(value_to_graphml_type(&Value::Float64(0.0)), "double");
        assert_eq!(value_to_graphml_type(&Value::Bool(true)), "boolean");
        assert_eq!(value_to_graphml_type(&Value::String("".into())), "string");
    }
}
