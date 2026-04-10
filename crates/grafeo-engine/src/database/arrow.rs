//! Arrow IPC export for query results.
//!
//! Converts [`QueryResult`](super::QueryResult) to Arrow [`RecordBatch`] and serializes to Arrow IPC format.
//! Feature-gated behind `arrow-export`.

use std::sync::Arc;

use arrow_array::Array;
use arrow_array::builder::{
    BinaryBuilder, BooleanBuilder, Float32Builder, Float64Builder, Int64Builder, StringBuilder,
};
use arrow_array::{ArrayRef, FixedSizeListArray, RecordBatch};
use arrow_ipc::writer::StreamWriter;
use arrow_schema::{ArrowError, DataType, Field, Schema, TimeUnit};

use grafeo_common::{LogicalType, Value};

/// Errors from Arrow export operations.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ArrowExportError {
    /// Error from the Arrow library.
    #[error("Arrow error: {0}")]
    Arrow(#[from] ArrowError),
}

/// Maps a grafeo [`LogicalType`] to an Arrow [`DataType`].
///
/// Falls back to `Utf8` for types that have no direct Arrow equivalent.
fn logical_type_to_arrow(logical_type: &LogicalType) -> DataType {
    match logical_type {
        LogicalType::Null => DataType::Null,
        LogicalType::Bool => DataType::Boolean,
        LogicalType::Int8 | LogicalType::Int16 | LogicalType::Int32 | LogicalType::Int64 => {
            DataType::Int64
        }
        LogicalType::Float32 | LogicalType::Float64 => DataType::Float64,
        LogicalType::String => DataType::Utf8,
        LogicalType::Bytes => DataType::Binary,
        LogicalType::Timestamp => DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
        LogicalType::Date => DataType::Date32,
        LogicalType::Time => DataType::Time64(TimeUnit::Nanosecond),
        LogicalType::Duration => DataType::Utf8, // ISO 8601 string (Arrow Duration lacks months)
        LogicalType::ZonedDatetime | LogicalType::ZonedTime => {
            DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into()))
        }
        LogicalType::Vector(dim) => DataType::FixedSizeList(
            Arc::new(Field::new("item", DataType::Float32, false)),
            i32::try_from(*dim).unwrap_or(0),
        ),
        LogicalType::List(_)
        | LogicalType::Map { .. }
        | LogicalType::Struct(_)
        | LogicalType::Node
        | LogicalType::Edge
        | LogicalType::Path
        | LogicalType::Any => DataType::Utf8,
        _ => DataType::Utf8,
    }
}

/// Infers the Arrow [`DataType`] for a column from its [`LogicalType`] hint and actual values.
///
/// If the logical type is `Any` (unknown), scans values to find the dominant type.
/// Falls back to `Utf8` for heterogeneous columns.
fn infer_column_type(logical_type: &LogicalType, column: &[&Value]) -> DataType {
    if *logical_type != LogicalType::Any {
        return logical_type_to_arrow(logical_type);
    }

    // Scan values to find the dominant non-null type
    let mut seen_type: Option<DataType> = None;
    for value in column {
        let dt = match value {
            Value::Null => continue,
            Value::Bool(_) => DataType::Boolean,
            Value::Int64(_) => DataType::Int64,
            Value::Float64(_) => DataType::Float64,
            Value::String(_) => DataType::Utf8,
            Value::Bytes(_) => DataType::Binary,
            Value::Timestamp(_) => DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
            Value::Date(_) => DataType::Date32,
            Value::Time(_) => DataType::Time64(TimeUnit::Nanosecond),
            Value::Duration(_) => DataType::Utf8,
            Value::ZonedDatetime(_) => {
                DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into()))
            }
            Value::Vector(v) => DataType::FixedSizeList(
                Arc::new(Field::new("item", DataType::Float32, false)),
                i32::try_from(v.len()).unwrap_or(0),
            ),
            Value::List(_)
            | Value::Map(_)
            | Value::Path { .. }
            | Value::GCounter(_)
            | Value::OnCounter { .. } => DataType::Utf8,
            _ => DataType::Utf8,
        };

        match &seen_type {
            None => seen_type = Some(dt),
            Some(existing) if *existing == dt => {}
            Some(_) => return DataType::Utf8, // Mixed types: fall back to string
        }
    }

    seen_type.unwrap_or(DataType::Null)
}

/// Builds an Arrow [`ArrayRef`] from a column of [`Value`] references.
fn build_array(column: &[&Value], target_type: &DataType) -> Result<ArrayRef, ArrowExportError> {
    let len = column.len();

    match target_type {
        DataType::Null => Ok(Arc::new(arrow_array::NullArray::new(len)) as ArrayRef),
        DataType::Boolean => {
            let mut builder = BooleanBuilder::with_capacity(len);
            for value in column {
                match value {
                    Value::Bool(b) => builder.append_value(*b),
                    Value::Null => builder.append_null(),
                    _ => builder.append_null(),
                }
            }
            Ok(Arc::new(builder.finish()) as ArrayRef)
        }
        DataType::Int64 => {
            let mut builder = Int64Builder::with_capacity(len);
            for value in column {
                match value {
                    Value::Int64(i) => builder.append_value(*i),
                    Value::Float64(f) => builder.append_value(*f as i64),
                    Value::Null => builder.append_null(),
                    _ => builder.append_null(),
                }
            }
            Ok(Arc::new(builder.finish()) as ArrayRef)
        }
        DataType::Float64 => {
            let mut builder = Float64Builder::with_capacity(len);
            for value in column {
                match value {
                    Value::Float64(f) => builder.append_value(*f),
                    Value::Int64(i) => builder.append_value(*i as f64),
                    Value::Null => builder.append_null(),
                    _ => builder.append_null(),
                }
            }
            Ok(Arc::new(builder.finish()) as ArrayRef)
        }
        DataType::Utf8 => {
            let mut builder = StringBuilder::with_capacity(len, len * 32);
            for value in column {
                match value {
                    Value::Null => builder.append_null(),
                    Value::String(s) => builder.append_value(s.as_str()),
                    other => builder.append_value(other.to_string()),
                }
            }
            Ok(Arc::new(builder.finish()) as ArrayRef)
        }
        DataType::Binary => {
            let mut builder = BinaryBuilder::with_capacity(len, len * 64);
            for value in column {
                match value {
                    Value::Bytes(b) => builder.append_value(b.as_ref()),
                    Value::Null => builder.append_null(),
                    _ => builder.append_null(),
                }
            }
            Ok(Arc::new(builder.finish()) as ArrayRef)
        }
        DataType::Timestamp(TimeUnit::Microsecond, _) => {
            let mut builder = Int64Builder::with_capacity(len);
            for value in column {
                match value {
                    Value::Timestamp(ts) => builder.append_value(ts.as_micros()),
                    Value::ZonedDatetime(zdt) => {
                        builder.append_value(zdt.as_timestamp().as_micros());
                    }
                    Value::Null => builder.append_null(),
                    _ => builder.append_null(),
                }
            }
            let int_array = builder.finish();
            // Reinterpret as TimestampMicrosecondArray
            let data = int_array.into_data();
            let ts_data = data
                .into_builder()
                .data_type(DataType::Timestamp(
                    TimeUnit::Microsecond,
                    Some("UTC".into()),
                ))
                .build()?;
            Ok(Arc::new(arrow_array::TimestampMicrosecondArray::from(ts_data)) as ArrayRef)
        }
        DataType::Date32 => {
            let values: Vec<Option<i32>> = column
                .iter()
                .map(|v| match v {
                    Value::Date(d) => Some(d.as_days()),
                    _ => None,
                })
                .collect();
            Ok(Arc::new(arrow_array::Date32Array::from(values)) as ArrayRef)
        }
        DataType::Time64(TimeUnit::Nanosecond) => {
            let mut builder = Int64Builder::with_capacity(len);
            for value in column {
                match value {
                    Value::Time(t) => builder.append_value(t.as_nanos() as i64),
                    Value::Null => builder.append_null(),
                    _ => builder.append_null(),
                }
            }
            let int_array = builder.finish();
            let data = int_array
                .into_data()
                .into_builder()
                .data_type(DataType::Time64(TimeUnit::Nanosecond))
                .build()?;
            Ok(Arc::new(arrow_array::Time64NanosecondArray::from(data)) as ArrayRef)
        }
        DataType::FixedSizeList(_, dim) => {
            let dim_usize = *dim as usize;
            let mut float_builder = Float32Builder::with_capacity(len * dim_usize);
            let mut null_mask = Vec::with_capacity(len);
            for value in column {
                match value {
                    Value::Vector(v) if v.len() == dim_usize => {
                        for f in v.iter() {
                            float_builder.append_value(*f);
                        }
                        null_mask.push(true);
                    }
                    Value::Null => {
                        for _ in 0..dim_usize {
                            float_builder.append_value(0.0);
                        }
                        null_mask.push(false);
                    }
                    _ => {
                        for _ in 0..dim_usize {
                            float_builder.append_value(0.0);
                        }
                        null_mask.push(false);
                    }
                }
            }
            let values_array = float_builder.finish();
            let field = Arc::new(Field::new("item", DataType::Float32, false));
            let list_array = FixedSizeListArray::try_new(
                field,
                *dim,
                Arc::new(values_array),
                Some(null_mask.into()),
            )?;
            Ok(Arc::new(list_array) as ArrayRef)
        }
        // Fallback: serialize as string
        _ => {
            let mut builder = StringBuilder::with_capacity(len, len * 32);
            for value in column {
                match value {
                    Value::Null => builder.append_null(),
                    other => builder.append_value(other.to_string()),
                }
            }
            Ok(Arc::new(builder.finish()) as ArrayRef)
        }
    }
}

/// Converts a [`QueryResult`](super::QueryResult) to an Arrow [`RecordBatch`].
///
/// # Errors
///
/// Returns [`ArrowExportError`] if column type inference fails or Arrow
/// array construction encounters incompatible data.
pub fn query_result_to_record_batch(
    columns: &[String],
    column_types: &[LogicalType],
    rows: &[Vec<Value>],
) -> Result<RecordBatch, ArrowExportError> {
    if columns.is_empty() {
        let schema = Arc::new(Schema::empty());
        return Ok(RecordBatch::new_empty(schema));
    }

    let num_cols = columns.len();
    let num_rows = rows.len();

    // Extract column-oriented data
    let mut col_values: Vec<Vec<&Value>> = vec![Vec::with_capacity(num_rows); num_cols];
    for row in rows {
        for (col_idx, value) in row.iter().enumerate() {
            if col_idx < num_cols {
                col_values[col_idx].push(value);
            }
        }
    }

    // Infer types and build arrays
    let mut fields = Vec::with_capacity(num_cols);
    let mut arrays: Vec<ArrayRef> = Vec::with_capacity(num_cols);

    for (col_idx, col_name) in columns.iter().enumerate() {
        let logical_type = column_types.get(col_idx).unwrap_or(&LogicalType::Any);
        let values = &col_values[col_idx];
        let arrow_type = infer_column_type(logical_type, values);

        fields.push(Field::new(col_name.as_str(), arrow_type.clone(), true));
        arrays.push(build_array(values, &arrow_type)?);
    }

    let schema = Arc::new(Schema::new(fields));
    Ok(RecordBatch::try_new(schema, arrays)?)
}

/// Serializes a [`RecordBatch`] to Arrow IPC stream format bytes.
///
/// # Errors
///
/// Returns [`ArrowExportError`] if IPC stream encoding fails.
pub fn record_batch_to_ipc_stream(batch: &RecordBatch) -> Result<Vec<u8>, ArrowExportError> {
    let mut buf = Vec::new();
    {
        let mut writer = StreamWriter::try_new(&mut buf, &batch.schema())?;
        writer.write(batch)?;
        writer.finish()?;
    }
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Arc as StdArc;

    use grafeo_common::PropertyKey;
    use grafeo_common::types::{Date, Duration, Time, Timestamp, ZonedDatetime};

    fn make_result(
        columns: Vec<&str>,
        types: Vec<LogicalType>,
        rows: Vec<Vec<Value>>,
    ) -> (Vec<String>, Vec<LogicalType>, Vec<Vec<Value>>) {
        (columns.into_iter().map(String::from).collect(), types, rows)
    }

    #[test]
    fn test_empty_result() {
        let (cols, types, rows) = make_result(vec![], vec![], vec![]);
        let batch = query_result_to_record_batch(&cols, &types, &rows).unwrap();
        assert_eq!(batch.num_columns(), 0);
        assert_eq!(batch.num_rows(), 0);
    }

    #[test]
    fn test_null_column() {
        let (cols, types, rows) = make_result(
            vec!["x"],
            vec![LogicalType::Null],
            vec![vec![Value::Null], vec![Value::Null]],
        );
        let batch = query_result_to_record_batch(&cols, &types, &rows).unwrap();
        assert_eq!(batch.num_rows(), 2);
        assert_eq!(*batch.schema().field(0).data_type(), DataType::Null);
    }

    #[test]
    fn test_bool_column() {
        let (cols, types, rows) = make_result(
            vec!["flag"],
            vec![LogicalType::Bool],
            vec![vec![Value::Bool(true)], vec![Value::Bool(false)]],
        );
        let batch = query_result_to_record_batch(&cols, &types, &rows).unwrap();
        let arr = batch
            .column(0)
            .as_any()
            .downcast_ref::<arrow_array::BooleanArray>()
            .unwrap();
        assert!(arr.value(0));
        assert!(!arr.value(1));
    }

    #[test]
    fn test_int64_column() {
        let (cols, types, rows) = make_result(
            vec!["age"],
            vec![LogicalType::Int64],
            vec![
                vec![Value::Int64(30)],
                vec![Value::Null],
                vec![Value::Int64(-5)],
            ],
        );
        let batch = query_result_to_record_batch(&cols, &types, &rows).unwrap();
        let arr = batch
            .column(0)
            .as_any()
            .downcast_ref::<arrow_array::Int64Array>()
            .unwrap();
        assert_eq!(arr.value(0), 30);
        assert!(arr.is_null(1));
        assert_eq!(arr.value(2), -5);
    }

    #[test]
    fn test_float64_column() {
        let (cols, types, rows) = make_result(
            vec!["score"],
            vec![LogicalType::Float64],
            vec![vec![Value::Float64(3.125)], vec![Value::Float64(-0.5)]],
        );
        let batch = query_result_to_record_batch(&cols, &types, &rows).unwrap();
        let arr = batch
            .column(0)
            .as_any()
            .downcast_ref::<arrow_array::Float64Array>()
            .unwrap();
        assert!((arr.value(0) - 3.125).abs() < f64::EPSILON);
    }

    #[test]
    fn test_string_column() {
        let (cols, types, rows) = make_result(
            vec!["name"],
            vec![LogicalType::String],
            vec![
                vec![Value::String("Alix".into())],
                vec![Value::Null],
                vec![Value::String("Gus".into())],
            ],
        );
        let batch = query_result_to_record_batch(&cols, &types, &rows).unwrap();
        let arr = batch
            .column(0)
            .as_any()
            .downcast_ref::<arrow_array::StringArray>()
            .unwrap();
        assert_eq!(arr.value(0), "Alix");
        assert!(arr.is_null(1));
        assert_eq!(arr.value(2), "Gus");
    }

    #[test]
    fn test_bytes_column() {
        let (cols, types, rows) = make_result(
            vec!["data"],
            vec![LogicalType::Bytes],
            vec![vec![Value::Bytes(StdArc::from(vec![1u8, 2, 3].as_slice()))]],
        );
        let batch = query_result_to_record_batch(&cols, &types, &rows).unwrap();
        let arr = batch
            .column(0)
            .as_any()
            .downcast_ref::<arrow_array::BinaryArray>()
            .unwrap();
        assert_eq!(arr.value(0), &[1, 2, 3]);
    }

    #[test]
    fn test_timestamp_column() {
        let ts = Timestamp::from_micros(1_700_000_000_000_000);
        let (cols, types, rows) = make_result(
            vec!["created"],
            vec![LogicalType::Timestamp],
            vec![vec![Value::Timestamp(ts)]],
        );
        let batch = query_result_to_record_batch(&cols, &types, &rows).unwrap();
        let arr = batch
            .column(0)
            .as_any()
            .downcast_ref::<arrow_array::TimestampMicrosecondArray>()
            .unwrap();
        assert_eq!(arr.value(0), 1_700_000_000_000_000);
    }

    #[test]
    fn test_date_column() {
        let date = Date::from_ymd(2025, 6, 15).unwrap();
        let (cols, types, rows) = make_result(
            vec!["birthday"],
            vec![LogicalType::Date],
            vec![vec![Value::Date(date)]],
        );
        let batch = query_result_to_record_batch(&cols, &types, &rows).unwrap();
        assert_eq!(batch.num_rows(), 1);
    }

    #[test]
    fn test_time_column() {
        let time = Time::from_hms(14, 30, 0).unwrap();
        let (cols, types, rows) = make_result(
            vec!["alarm"],
            vec![LogicalType::Time],
            vec![vec![Value::Time(time)]],
        );
        let batch = query_result_to_record_batch(&cols, &types, &rows).unwrap();
        assert_eq!(batch.num_rows(), 1);
    }

    #[test]
    fn test_duration_as_string() {
        let dur = Duration::new(2, 5, 1_000_000_000);
        let (cols, types, rows) = make_result(
            vec!["interval"],
            vec![LogicalType::Duration],
            vec![vec![Value::Duration(dur)]],
        );
        let batch = query_result_to_record_batch(&cols, &types, &rows).unwrap();
        // Duration maps to Utf8
        assert_eq!(*batch.schema().field(0).data_type(), DataType::Utf8);
    }

    #[test]
    fn test_zoned_datetime_column() {
        let zdt = ZonedDatetime::from_timestamp_offset(
            Timestamp::from_micros(1_700_000_000_000_000),
            3600,
        );
        let (cols, types, rows) = make_result(
            vec!["event_at"],
            vec![LogicalType::ZonedDatetime],
            vec![vec![Value::ZonedDatetime(zdt)]],
        );
        let batch = query_result_to_record_batch(&cols, &types, &rows).unwrap();
        let arr = batch
            .column(0)
            .as_any()
            .downcast_ref::<arrow_array::TimestampMicrosecondArray>()
            .unwrap();
        assert_eq!(arr.value(0), 1_700_000_000_000_000);
    }

    #[test]
    fn test_vector_column() {
        let vec3 = Value::Vector(StdArc::from(vec![1.0f32, 2.0, 3.0].as_slice()));
        let (cols, types, rows) = make_result(
            vec!["embedding"],
            vec![LogicalType::Vector(3)],
            vec![vec![vec3]],
        );
        let batch = query_result_to_record_batch(&cols, &types, &rows).unwrap();
        assert_eq!(batch.num_rows(), 1);
        match batch.schema().field(0).data_type() {
            DataType::FixedSizeList(_, 3) => {}
            other => panic!("Expected FixedSizeList(_, 3), got {other:?}"),
        }
    }

    #[test]
    fn test_list_as_string() {
        let list = Value::List(StdArc::from(vec![Value::Int64(1), Value::Int64(2)]));
        let (cols, types, rows) = make_result(
            vec!["items"],
            vec![LogicalType::List(Box::new(LogicalType::Int64))],
            vec![vec![list]],
        );
        let batch = query_result_to_record_batch(&cols, &types, &rows).unwrap();
        assert_eq!(*batch.schema().field(0).data_type(), DataType::Utf8);
    }

    #[test]
    fn test_map_as_string() {
        let mut map = BTreeMap::new();
        map.insert(PropertyKey::from("key"), Value::String("val".into()));
        let map_val = Value::Map(StdArc::from(map));
        let (cols, types, rows) = make_result(
            vec!["props"],
            vec![LogicalType::Map {
                key: Box::new(LogicalType::String),
                value: Box::new(LogicalType::String),
            }],
            vec![vec![map_val]],
        );
        let batch = query_result_to_record_batch(&cols, &types, &rows).unwrap();
        assert_eq!(*batch.schema().field(0).data_type(), DataType::Utf8);
    }

    #[test]
    fn test_heterogeneous_column_falls_back_to_string() {
        let (cols, types, rows) = make_result(
            vec!["mixed"],
            vec![LogicalType::Any],
            vec![vec![Value::Int64(42)], vec![Value::String("hello".into())]],
        );
        let batch = query_result_to_record_batch(&cols, &types, &rows).unwrap();
        assert_eq!(*batch.schema().field(0).data_type(), DataType::Utf8);
    }

    #[test]
    fn test_multi_column() {
        let (cols, types, rows) = make_result(
            vec!["name", "age", "active"],
            vec![LogicalType::String, LogicalType::Int64, LogicalType::Bool],
            vec![
                vec![
                    Value::String("Alix".into()),
                    Value::Int64(30),
                    Value::Bool(true),
                ],
                vec![
                    Value::String("Gus".into()),
                    Value::Int64(25),
                    Value::Bool(false),
                ],
            ],
        );
        let batch = query_result_to_record_batch(&cols, &types, &rows).unwrap();
        assert_eq!(batch.num_columns(), 3);
        assert_eq!(batch.num_rows(), 2);
    }

    #[test]
    fn test_ipc_roundtrip() {
        let (cols, types, rows) = make_result(
            vec!["id", "name"],
            vec![LogicalType::Int64, LogicalType::String],
            vec![
                vec![Value::Int64(1), Value::String("Alix".into())],
                vec![Value::Int64(2), Value::String("Gus".into())],
            ],
        );
        let batch = query_result_to_record_batch(&cols, &types, &rows).unwrap();
        let ipc_bytes = record_batch_to_ipc_stream(&batch).unwrap();
        assert!(!ipc_bytes.is_empty());

        // Read back
        let cursor = std::io::Cursor::new(ipc_bytes);
        let reader = arrow_ipc::reader::StreamReader::try_new(cursor, None).unwrap();
        let batches: Vec<_> = reader.into_iter().map(|b| b.unwrap()).collect();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].num_rows(), 2);
        assert_eq!(batches[0].num_columns(), 2);
    }
}
