//! CSV and JSON Lines import commands.

use std::io::{BufRead, BufReader};
use std::path::Path;

use anyhow::{Context, Result};

use crate::output;
use crate::{ImportCommands, OutputFormat};

/// Run an import subcommand.
pub fn run(cmd: ImportCommands, _format: OutputFormat, quiet: bool) -> Result<()> {
    match cmd {
        ImportCommands::Csv {
            file,
            path,
            headers,
            separator,
            label,
        } => import_csv(&file, &path, headers, separator.as_deref(), &label, quiet),
        ImportCommands::Jsonl { file, path, label } => import_jsonl(&file, &path, &label, quiet),
    }
}

/// Build a `LOAD DATA` query for CSV import and execute it via the engine.
fn import_csv(
    file: &Path,
    db_path: &Path,
    headers: bool,
    separator: Option<&str>,
    label: &str,
    quiet: bool,
) -> Result<()> {
    // Canonicalize file path so the engine can find it
    let abs_path = file
        .canonicalize()
        .with_context(|| format!("File not found: {}", file.display()))?;
    let path_str = abs_path.to_string_lossy().replace('\\', "/");

    // Read column headers from the first line so we can build the property map
    let columns = if headers {
        let f = std::fs::File::open(&abs_path)
            .with_context(|| format!("Failed to open file: {}", abs_path.display()))?;
        let mut reader = BufReader::new(f);
        let mut header_line = String::new();
        reader
            .read_line(&mut header_line)
            .context("Failed to read header line")?;
        let sep = separator.unwrap_or(",");
        let sep_char = parse_separator(sep)?;
        header_line
            .trim()
            .split(sep_char)
            .map(|h| h.trim().trim_matches('"').to_string())
            .filter(|h| !h.is_empty())
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    // Build the LOAD DATA query
    let header_clause = if headers { " WITH HEADERS" } else { "" };
    let separator_clause = separator
        .map(|s| format!(" FIELDTERMINATOR '{}'", s.replace('\'', "\\'")))
        .unwrap_or_default();

    let insert_clause = if headers && !columns.is_empty() {
        let props = columns
            .iter()
            .map(|col| {
                let safe_col = sanitize_identifier(col);
                format!("{safe_col}: row.{safe_col}")
            })
            .collect::<Vec<_>>()
            .join(", ");
        format!("INSERT (:{label} {{{props}}})")
    } else {
        // Without headers, we cannot map columns by name.
        // Insert nodes with no properties (positional access is limited).
        format!("INSERT (:{label} {{}})")
    };

    let query = format!(
        "LOAD DATA FROM '{path_str}' FORMAT CSV{header_clause} AS row{separator_clause} {insert_clause}"
    );

    // Open or create the database and execute
    let db = open_or_create(db_path)?;
    let session = db.session();
    session
        .execute(&query)
        .with_context(|| "Import query failed")?;

    // Count imported nodes
    let count_result = session
        .execute(&format!("MATCH (n:{label}) RETURN count(n) AS c"))
        .context("Failed to count imported nodes")?;

    let count = count_result
        .rows()
        .first()
        .and_then(|row| row.first())
        .and_then(|v| match v {
            grafeo_common::types::Value::Int64(n) => Some(*n),
            _ => None,
        })
        .unwrap_or(0);

    output::status(
        &format!(
            "Imported {count} nodes with label '{label}' from {}",
            file.display()
        ),
        quiet,
    );
    Ok(())
}

/// Build a `LOAD DATA` query for JSONL import and execute it via the engine.
fn import_jsonl(file: &Path, db_path: &Path, label: &str, quiet: bool) -> Result<()> {
    let abs_path = file
        .canonicalize()
        .with_context(|| format!("File not found: {}", file.display()))?;
    let path_str = abs_path.to_string_lossy().replace('\\', "/");

    // Read the first line to discover JSON keys for property mapping
    let keys = {
        let f = std::fs::File::open(&abs_path)
            .with_context(|| format!("Failed to open file: {}", abs_path.display()))?;
        let reader = BufReader::new(f);
        let mut keys = Vec::new();
        for line in reader.lines() {
            let line = line.context("Failed to read JSONL file")?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(obj) =
                serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(trimmed)
            {
                keys = obj.keys().cloned().collect();
            }
            break;
        }
        keys
    };

    let insert_clause = if keys.is_empty() {
        format!("INSERT (:{label} {{}})")
    } else {
        let props = keys
            .iter()
            .map(|key| {
                let safe_key = sanitize_identifier(key);
                format!("{safe_key}: row.{safe_key}")
            })
            .collect::<Vec<_>>()
            .join(", ");
        format!("INSERT (:{label} {{{props}}})")
    };

    let query = format!("LOAD DATA FROM '{path_str}' FORMAT JSONL AS row {insert_clause}");

    let db = open_or_create(db_path)?;
    let session = db.session();
    session
        .execute(&query)
        .with_context(|| "Import query failed")?;

    let count_result = session
        .execute(&format!("MATCH (n:{label}) RETURN count(n) AS c"))
        .context("Failed to count imported nodes")?;

    let count = count_result
        .rows()
        .first()
        .and_then(|row| row.first())
        .and_then(|v| match v {
            grafeo_common::types::Value::Int64(n) => Some(*n),
            _ => None,
        })
        .unwrap_or(0);

    output::status(
        &format!(
            "Imported {count} nodes with label '{label}' from {}",
            file.display()
        ),
        quiet,
    );
    Ok(())
}

/// Open an existing database or create a new one at the given path.
fn open_or_create(path: &Path) -> Result<grafeo_engine::GrafeoDB> {
    if path.exists() {
        super::open_existing(path)
    } else {
        grafeo_engine::GrafeoDB::open(path)
            .with_context(|| format!("Failed to create database at {}", path.display()))
    }
}

/// Parse a separator string into a single character.
fn parse_separator(s: &str) -> Result<char> {
    match s {
        "\\t" | "TAB" | "tab" => Ok('\t'),
        _ if s.len() == 1 => Ok(s.chars().next().unwrap()),
        _ => anyhow::bail!("Separator must be a single character, got: '{s}'"),
    }
}

/// Sanitize a column/key name for use as a GQL identifier.
///
/// Strips characters that are not alphanumeric or underscores.
/// If the result is empty, returns a placeholder.
fn sanitize_identifier(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.is_empty() {
        "_col".to_string()
    } else {
        sanitized
    }
}
