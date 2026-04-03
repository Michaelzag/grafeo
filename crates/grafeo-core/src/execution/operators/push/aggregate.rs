//! Push-based aggregate operator (pipeline breaker).

use crate::execution::chunk::DataChunk;
use crate::execution::operators::OperatorError;
use crate::execution::operators::accumulator::{AggregateExpr, AggregateFunction};
use crate::execution::pipeline::{ChunkSizeHint, PushOperator, Sink};
#[cfg(feature = "spill")]
use crate::execution::spill::{PartitionedState, SpillManager};
use crate::execution::vector::ValueVector;
use grafeo_common::types::Value;
use std::collections::HashMap;
#[cfg(feature = "spill")]
use std::io::{Read, Write};
#[cfg(feature = "spill")]
use std::sync::Arc;

/// Accumulator for aggregate state.
#[derive(Debug, Clone, Default)]
struct Accumulator {
    count: i64,
    sum: f64,
    min: Option<Value>,
    max: Option<Value>,
    first: Option<Value>,
}

impl Accumulator {
    fn new() -> Self {
        Self {
            count: 0,
            sum: 0.0,
            min: None,
            max: None,
            first: None,
        }
    }

    fn add(&mut self, value: &Value) {
        // Skip nulls for aggregates
        if matches!(value, Value::Null) {
            return;
        }

        self.count += 1;

        // Sum (for numeric types)
        if let Some(n) = value_to_f64(value) {
            self.sum += n;
        }

        // Min
        if self.min.is_none() || compare_for_min(&self.min, value) {
            self.min = Some(value.clone());
        }

        // Max
        if self.max.is_none() || compare_for_max(&self.max, value) {
            self.max = Some(value.clone());
        }

        // First
        if self.first.is_none() {
            self.first = Some(value.clone());
        }
    }

    fn finalize(&mut self, func: AggregateFunction) -> Value {
        match func {
            AggregateFunction::Count | AggregateFunction::CountNonNull => Value::Int64(self.count),
            AggregateFunction::Sum => {
                if self.count == 0 {
                    Value::Null
                } else {
                    Value::Float64(self.sum)
                }
            }
            AggregateFunction::Min => self.min.take().unwrap_or(Value::Null),
            AggregateFunction::Max => self.max.take().unwrap_or(Value::Null),
            AggregateFunction::Avg => {
                if self.count == 0 {
                    Value::Null
                } else {
                    Value::Float64(self.sum / self.count as f64)
                }
            }
            AggregateFunction::First => self.first.take().unwrap_or(Value::Null),
            // Advanced functions not supported in push-based accumulator;
            // these are handled by the pull-based AggregateState instead.
            AggregateFunction::Last
            | AggregateFunction::Collect
            | AggregateFunction::StdDev
            | AggregateFunction::StdDevPop
            | AggregateFunction::Variance
            | AggregateFunction::VariancePop
            | AggregateFunction::PercentileDisc
            | AggregateFunction::PercentileCont
            | AggregateFunction::GroupConcat
            | AggregateFunction::Sample
            | AggregateFunction::CovarSamp
            | AggregateFunction::CovarPop
            | AggregateFunction::Corr
            | AggregateFunction::RegrSlope
            | AggregateFunction::RegrIntercept
            | AggregateFunction::RegrR2
            | AggregateFunction::RegrCount
            | AggregateFunction::RegrSxx
            | AggregateFunction::RegrSyy
            | AggregateFunction::RegrSxy
            | AggregateFunction::RegrAvgx
            | AggregateFunction::RegrAvgy => Value::Null,
        }
    }
}

use crate::execution::operators::value_utils::{
    is_greater_than as compare_for_max, is_less_than as compare_for_min, value_to_f64,
};

/// Hash key for grouping.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct GroupKey(Vec<u64>);

impl GroupKey {
    fn from_row(chunk: &DataChunk, row: usize, group_by: &[usize]) -> Self {
        let hashes: Vec<u64> = group_by
            .iter()
            .map(|&col| {
                chunk
                    .column(col)
                    .and_then(|c| c.get_value(row))
                    .map_or(0, |v| hash_value(&v))
            })
            .collect();
        Self(hashes)
    }
}

fn hash_value(value: &Value) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    // Discriminant tag prevents cross-type collisions (e.g. Null vs unknown)
    match value {
        Value::Null => 0u8.hash(&mut hasher),
        Value::Bool(b) => {
            1u8.hash(&mut hasher);
            b.hash(&mut hasher);
        }
        Value::Int64(i) => {
            2u8.hash(&mut hasher);
            i.hash(&mut hasher);
        }
        Value::Float64(f) => {
            3u8.hash(&mut hasher);
            f.to_bits().hash(&mut hasher);
        }
        Value::String(s) => {
            4u8.hash(&mut hasher);
            s.hash(&mut hasher);
        }
        Value::Bytes(b) => {
            5u8.hash(&mut hasher);
            b.hash(&mut hasher);
        }
        Value::Timestamp(t) => {
            6u8.hash(&mut hasher);
            t.hash(&mut hasher);
        }
        Value::Date(d) => {
            7u8.hash(&mut hasher);
            d.hash(&mut hasher);
        }
        Value::Time(t) => {
            8u8.hash(&mut hasher);
            t.hash(&mut hasher);
        }
        Value::Duration(d) => {
            9u8.hash(&mut hasher);
            d.hash(&mut hasher);
        }
        Value::ZonedDatetime(zdt) => {
            10u8.hash(&mut hasher);
            zdt.hash(&mut hasher);
        }
        Value::List(list) => {
            11u8.hash(&mut hasher);
            list.len().hash(&mut hasher);
            for elem in list.iter() {
                hash_value(elem).hash(&mut hasher);
            }
        }
        Value::Map(map) => {
            12u8.hash(&mut hasher);
            map.len().hash(&mut hasher);
            // BTreeMap iterates in key order, so hashing is deterministic
            for (k, v) in map.as_ref() {
                k.as_str().hash(&mut hasher);
                hash_value(v).hash(&mut hasher);
            }
        }
        Value::Vector(vec) => {
            13u8.hash(&mut hasher);
            vec.len().hash(&mut hasher);
            for f in vec.iter() {
                f.to_bits().hash(&mut hasher);
            }
        }
        Value::Path { nodes, edges } => {
            14u8.hash(&mut hasher);
            nodes.len().hash(&mut hasher);
            for n in nodes.iter() {
                hash_value(n).hash(&mut hasher);
            }
            for e in edges.iter() {
                hash_value(e).hash(&mut hasher);
            }
        }
        Value::GCounter(map) => {
            15u8.hash(&mut hasher);
            let mut entries: Vec<_> = map.iter().collect();
            entries.sort_by_key(|(k, _)| *k);
            for (k, v) in entries {
                k.hash(&mut hasher);
                v.hash(&mut hasher);
            }
        }
        Value::OnCounter { pos, neg } => {
            16u8.hash(&mut hasher);
            let mut pos_entries: Vec<_> = pos.iter().collect();
            pos_entries.sort_by_key(|(k, _)| *k);
            for (k, v) in pos_entries {
                k.hash(&mut hasher);
                v.hash(&mut hasher);
            }
            let mut neg_entries: Vec<_> = neg.iter().collect();
            neg_entries.sort_by_key(|(k, _)| *k);
            for (k, v) in neg_entries {
                k.hash(&mut hasher);
                v.hash(&mut hasher);
            }
        }
    }
    hasher.finish()
}

/// Group state with key values and accumulators.
#[derive(Clone)]
struct GroupState {
    key_values: Vec<Value>,
    accumulators: Vec<Accumulator>,
}

/// Push-based aggregate operator.
///
/// This is a pipeline breaker that accumulates all input, groups by key,
/// and produces aggregated output in the finalize phase.
pub struct AggregatePushOperator {
    /// Columns to group by.
    group_by: Vec<usize>,
    /// Aggregate expressions.
    aggregates: Vec<AggregateExpr>,
    /// Group states by hash key.
    groups: HashMap<GroupKey, GroupState>,
    /// Global accumulator (for no GROUP BY).
    global_state: Option<Vec<Accumulator>>,
}

impl AggregatePushOperator {
    /// Create a new aggregate operator.
    pub fn new(group_by: Vec<usize>, aggregates: Vec<AggregateExpr>) -> Self {
        let global_state = if group_by.is_empty() {
            Some(aggregates.iter().map(|_| Accumulator::new()).collect())
        } else {
            None
        };

        Self {
            group_by,
            aggregates,
            groups: HashMap::new(),
            global_state,
        }
    }

    /// Create a simple global aggregate (no GROUP BY).
    pub fn global(aggregates: Vec<AggregateExpr>) -> Self {
        Self::new(Vec::new(), aggregates)
    }
}

impl PushOperator for AggregatePushOperator {
    fn push(&mut self, chunk: DataChunk, _sink: &mut dyn Sink) -> Result<bool, OperatorError> {
        if chunk.is_empty() {
            return Ok(true);
        }

        for row in chunk.selected_indices() {
            if self.group_by.is_empty() {
                // Global aggregation
                if let Some(ref mut accumulators) = self.global_state {
                    for (acc, expr) in accumulators.iter_mut().zip(&self.aggregates) {
                        if let Some(col) = expr.column {
                            if let Some(c) = chunk.column(col)
                                && let Some(val) = c.get_value(row)
                            {
                                acc.add(&val);
                            }
                        } else {
                            // COUNT(*)
                            acc.count += 1;
                        }
                    }
                }
            } else {
                // Group by aggregation
                let key = GroupKey::from_row(&chunk, row, &self.group_by);

                let state = self.groups.entry(key).or_insert_with(|| {
                    let key_values: Vec<Value> = self
                        .group_by
                        .iter()
                        .map(|&col| {
                            chunk
                                .column(col)
                                .and_then(|c| c.get_value(row))
                                .unwrap_or(Value::Null)
                        })
                        .collect();

                    GroupState {
                        key_values,
                        accumulators: self.aggregates.iter().map(|_| Accumulator::new()).collect(),
                    }
                });

                for (acc, expr) in state.accumulators.iter_mut().zip(&self.aggregates) {
                    if let Some(col) = expr.column {
                        if let Some(c) = chunk.column(col)
                            && let Some(val) = c.get_value(row)
                        {
                            acc.add(&val);
                        }
                    } else {
                        // COUNT(*)
                        acc.count += 1;
                    }
                }
            }
        }

        Ok(true)
    }

    fn finalize(&mut self, sink: &mut dyn Sink) -> Result<(), OperatorError> {
        let num_output_cols = self.group_by.len() + self.aggregates.len();
        let mut columns: Vec<ValueVector> =
            (0..num_output_cols).map(|_| ValueVector::new()).collect();

        if self.group_by.is_empty() {
            // Global aggregation - single row output
            if let Some(ref mut accumulators) = self.global_state {
                for (i, (acc, expr)) in accumulators.iter_mut().zip(&self.aggregates).enumerate() {
                    columns[i].push(acc.finalize(expr.function));
                }
            }
        } else {
            // Group by - one row per group
            for state in self.groups.values_mut() {
                // Output group key columns
                for (i, val) in state.key_values.iter().enumerate() {
                    columns[i].push(val.clone());
                }

                // Output aggregate results
                for (i, (acc, expr)) in state
                    .accumulators
                    .iter_mut()
                    .zip(&self.aggregates)
                    .enumerate()
                {
                    columns[self.group_by.len() + i].push(acc.finalize(expr.function));
                }
            }
        }

        if !columns.is_empty() && !columns[0].is_empty() {
            let chunk = DataChunk::new(columns);
            sink.consume(chunk)?;
        }

        Ok(())
    }

    fn preferred_chunk_size(&self) -> ChunkSizeHint {
        ChunkSizeHint::Default
    }

    fn name(&self) -> &'static str {
        "AggregatePush"
    }
}

/// Default spill threshold for aggregates (number of groups).
#[cfg(feature = "spill")]
pub const DEFAULT_AGGREGATE_SPILL_THRESHOLD: usize = 50_000;

/// Serializes a GroupState to bytes.
#[cfg(feature = "spill")]
fn serialize_group_state(state: &GroupState, w: &mut dyn Write) -> std::io::Result<()> {
    use crate::execution::spill::serialize_value;

    // Write key values
    w.write_all(&(state.key_values.len() as u64).to_le_bytes())?;
    for val in &state.key_values {
        serialize_value(val, w)?;
    }

    // Write accumulators
    w.write_all(&(state.accumulators.len() as u64).to_le_bytes())?;
    for acc in &state.accumulators {
        w.write_all(&acc.count.to_le_bytes())?;
        w.write_all(&acc.sum.to_bits().to_le_bytes())?;

        // Min
        let has_min = acc.min.is_some();
        w.write_all(&[has_min as u8])?;
        if let Some(ref v) = acc.min {
            serialize_value(v, w)?;
        }

        // Max
        let has_max = acc.max.is_some();
        w.write_all(&[has_max as u8])?;
        if let Some(ref v) = acc.max {
            serialize_value(v, w)?;
        }

        // First
        let has_first = acc.first.is_some();
        w.write_all(&[has_first as u8])?;
        if let Some(ref v) = acc.first {
            serialize_value(v, w)?;
        }
    }

    Ok(())
}

/// Deserializes a GroupState from bytes.
#[cfg(feature = "spill")]
fn deserialize_group_state(r: &mut dyn Read) -> std::io::Result<GroupState> {
    use crate::execution::spill::deserialize_value;

    // Read key values
    let mut len_buf = [0u8; 8];
    r.read_exact(&mut len_buf)?;
    let num_keys = u64::from_le_bytes(len_buf) as usize;

    let mut key_values = Vec::with_capacity(num_keys);
    for _ in 0..num_keys {
        key_values.push(deserialize_value(r)?);
    }

    // Read accumulators
    r.read_exact(&mut len_buf)?;
    let num_accumulators = u64::from_le_bytes(len_buf) as usize;

    let mut accumulators = Vec::with_capacity(num_accumulators);
    for _ in 0..num_accumulators {
        let mut count_buf = [0u8; 8];
        r.read_exact(&mut count_buf)?;
        let count = i64::from_le_bytes(count_buf);

        r.read_exact(&mut count_buf)?;
        let sum = f64::from_bits(u64::from_le_bytes(count_buf));

        // Min
        let mut flag_buf = [0u8; 1];
        r.read_exact(&mut flag_buf)?;
        let min = if flag_buf[0] != 0 {
            Some(deserialize_value(r)?)
        } else {
            None
        };

        // Max
        r.read_exact(&mut flag_buf)?;
        let max = if flag_buf[0] != 0 {
            Some(deserialize_value(r)?)
        } else {
            None
        };

        // First
        r.read_exact(&mut flag_buf)?;
        let first = if flag_buf[0] != 0 {
            Some(deserialize_value(r)?)
        } else {
            None
        };

        accumulators.push(Accumulator {
            count,
            sum,
            min,
            max,
            first,
        });
    }

    Ok(GroupState {
        key_values,
        accumulators,
    })
}

/// Push-based aggregate operator with spilling support.
///
/// Uses partitioned hash table that can spill cold partitions to disk
/// when memory pressure is high.
#[cfg(feature = "spill")]
pub struct SpillableAggregatePushOperator {
    /// Columns to group by.
    group_by: Vec<usize>,
    /// Aggregate expressions.
    aggregates: Vec<AggregateExpr>,
    /// Spill manager (None = no spilling).
    spill_manager: Option<Arc<SpillManager>>,
    /// Partitioned groups (used when spilling is enabled).
    partitioned_groups: Option<PartitionedState<GroupState>>,
    /// Non-partitioned groups (used when spilling is disabled).
    groups: HashMap<GroupKey, GroupState>,
    /// Global accumulator (for no GROUP BY).
    global_state: Option<Vec<Accumulator>>,
    /// Spill threshold (number of groups).
    spill_threshold: usize,
    /// Whether we've switched to partitioned mode.
    using_partitioned: bool,
}

#[cfg(feature = "spill")]
impl SpillableAggregatePushOperator {
    /// Create a new spillable aggregate operator.
    pub fn new(group_by: Vec<usize>, aggregates: Vec<AggregateExpr>) -> Self {
        let global_state = if group_by.is_empty() {
            Some(aggregates.iter().map(|_| Accumulator::new()).collect())
        } else {
            None
        };

        Self {
            group_by,
            aggregates,
            spill_manager: None,
            partitioned_groups: None,
            groups: HashMap::new(),
            global_state,
            spill_threshold: DEFAULT_AGGREGATE_SPILL_THRESHOLD,
            using_partitioned: false,
        }
    }

    /// Create a spillable aggregate operator with spilling enabled.
    pub fn with_spilling(
        group_by: Vec<usize>,
        aggregates: Vec<AggregateExpr>,
        manager: Arc<SpillManager>,
        threshold: usize,
    ) -> Self {
        let global_state = if group_by.is_empty() {
            Some(aggregates.iter().map(|_| Accumulator::new()).collect())
        } else {
            None
        };

        let partitioned = PartitionedState::new(
            Arc::clone(&manager),
            256, // Number of partitions
            serialize_group_state,
            deserialize_group_state,
        );

        Self {
            group_by,
            aggregates,
            spill_manager: Some(manager),
            partitioned_groups: Some(partitioned),
            groups: HashMap::new(),
            global_state,
            spill_threshold: threshold,
            using_partitioned: true,
        }
    }

    /// Create a simple global aggregate (no GROUP BY).
    pub fn global(aggregates: Vec<AggregateExpr>) -> Self {
        Self::new(Vec::new(), aggregates)
    }

    /// Sets the spill threshold.
    pub fn with_threshold(mut self, threshold: usize) -> Self {
        self.spill_threshold = threshold;
        self
    }

    /// Switches to partitioned mode if needed.
    fn maybe_spill(&mut self) -> Result<(), OperatorError> {
        if self.global_state.is_some() {
            // Global aggregation doesn't need spilling
            return Ok(());
        }

        // If using partitioned state, check if we need to spill
        if let Some(ref mut partitioned) = self.partitioned_groups {
            if partitioned.total_size() >= self.spill_threshold {
                partitioned
                    .spill_largest()
                    .map_err(|e| OperatorError::Execution(e.to_string()))?;
            }
        } else if self.groups.len() >= self.spill_threshold {
            // Not using partitioned state yet, but reached threshold
            // If spilling is configured, switch to partitioned mode
            if let Some(ref manager) = self.spill_manager {
                let mut partitioned = PartitionedState::new(
                    Arc::clone(manager),
                    256,
                    serialize_group_state,
                    deserialize_group_state,
                );

                // Move existing groups to partitioned state
                for (_key, state) in self.groups.drain() {
                    partitioned
                        .insert(state.key_values.clone(), state)
                        .map_err(|e| OperatorError::Execution(e.to_string()))?;
                }

                self.partitioned_groups = Some(partitioned);
                self.using_partitioned = true;
            }
        }

        Ok(())
    }
}

#[cfg(feature = "spill")]
impl PushOperator for SpillableAggregatePushOperator {
    fn push(&mut self, chunk: DataChunk, _sink: &mut dyn Sink) -> Result<bool, OperatorError> {
        if chunk.is_empty() {
            return Ok(true);
        }

        for row in chunk.selected_indices() {
            if self.group_by.is_empty() {
                // Global aggregation - same as non-spillable
                if let Some(ref mut accumulators) = self.global_state {
                    for (acc, expr) in accumulators.iter_mut().zip(&self.aggregates) {
                        if let Some(col) = expr.column {
                            if let Some(c) = chunk.column(col)
                                && let Some(val) = c.get_value(row)
                            {
                                acc.add(&val);
                            }
                        } else {
                            acc.count += 1;
                        }
                    }
                }
            } else if self.using_partitioned {
                // Use partitioned state
                if let Some(ref mut partitioned) = self.partitioned_groups {
                    let key_values: Vec<Value> = self
                        .group_by
                        .iter()
                        .map(|&col| {
                            chunk
                                .column(col)
                                .and_then(|c| c.get_value(row))
                                .unwrap_or(Value::Null)
                        })
                        .collect();

                    let aggregates = &self.aggregates;
                    let state = partitioned
                        .get_or_insert_with(key_values.clone(), || GroupState {
                            key_values: key_values.clone(),
                            accumulators: aggregates.iter().map(|_| Accumulator::new()).collect(),
                        })
                        .map_err(|e| OperatorError::Execution(e.to_string()))?;

                    for (acc, expr) in state.accumulators.iter_mut().zip(&self.aggregates) {
                        if let Some(col) = expr.column {
                            if let Some(c) = chunk.column(col)
                                && let Some(val) = c.get_value(row)
                            {
                                acc.add(&val);
                            }
                        } else {
                            acc.count += 1;
                        }
                    }
                }
            } else {
                // Use regular hash map
                let key = GroupKey::from_row(&chunk, row, &self.group_by);

                let state = self.groups.entry(key).or_insert_with(|| {
                    let key_values: Vec<Value> = self
                        .group_by
                        .iter()
                        .map(|&col| {
                            chunk
                                .column(col)
                                .and_then(|c| c.get_value(row))
                                .unwrap_or(Value::Null)
                        })
                        .collect();

                    GroupState {
                        key_values,
                        accumulators: self.aggregates.iter().map(|_| Accumulator::new()).collect(),
                    }
                });

                for (acc, expr) in state.accumulators.iter_mut().zip(&self.aggregates) {
                    if let Some(col) = expr.column {
                        if let Some(c) = chunk.column(col)
                            && let Some(val) = c.get_value(row)
                        {
                            acc.add(&val);
                        }
                    } else {
                        acc.count += 1;
                    }
                }
            }
        }

        // Check if we need to spill
        self.maybe_spill()?;

        Ok(true)
    }

    fn finalize(&mut self, sink: &mut dyn Sink) -> Result<(), OperatorError> {
        let num_output_cols = self.group_by.len() + self.aggregates.len();
        let mut columns: Vec<ValueVector> =
            (0..num_output_cols).map(|_| ValueVector::new()).collect();

        if self.group_by.is_empty() {
            // Global aggregation - single row output
            if let Some(ref mut accumulators) = self.global_state {
                for (i, (acc, expr)) in accumulators.iter_mut().zip(&self.aggregates).enumerate() {
                    columns[i].push(acc.finalize(expr.function));
                }
            }
        } else if self.using_partitioned {
            // Drain partitioned state
            if let Some(ref mut partitioned) = self.partitioned_groups {
                let groups = partitioned
                    .drain_all()
                    .map_err(|e| OperatorError::Execution(e.to_string()))?;

                for (_key, mut state) in groups {
                    // Output group key columns
                    for (i, val) in state.key_values.iter().enumerate() {
                        columns[i].push(val.clone());
                    }

                    // Output aggregate results
                    for (i, (acc, expr)) in state
                        .accumulators
                        .iter_mut()
                        .zip(&self.aggregates)
                        .enumerate()
                    {
                        columns[self.group_by.len() + i].push(acc.finalize(expr.function));
                    }
                }
            }
        } else {
            // Group by using regular hash map - one row per group
            for state in self.groups.values_mut() {
                // Output group key columns
                for (i, val) in state.key_values.iter().enumerate() {
                    columns[i].push(val.clone());
                }

                // Output aggregate results
                for (i, (acc, expr)) in state
                    .accumulators
                    .iter_mut()
                    .zip(&self.aggregates)
                    .enumerate()
                {
                    columns[self.group_by.len() + i].push(acc.finalize(expr.function));
                }
            }
        }

        if !columns.is_empty() && !columns[0].is_empty() {
            let chunk = DataChunk::new(columns);
            sink.consume(chunk)?;
        }

        Ok(())
    }

    fn preferred_chunk_size(&self) -> ChunkSizeHint {
        ChunkSizeHint::Default
    }

    fn name(&self) -> &'static str {
        "SpillableAggregatePush"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::sink::CollectorSink;

    fn create_test_chunk(values: &[i64]) -> DataChunk {
        let v: Vec<Value> = values.iter().map(|&i| Value::Int64(i)).collect();
        let vector = ValueVector::from_values(&v);
        DataChunk::new(vec![vector])
    }

    fn create_two_column_chunk(col1: &[i64], col2: &[i64]) -> DataChunk {
        let v1: Vec<Value> = col1.iter().map(|&i| Value::Int64(i)).collect();
        let v2: Vec<Value> = col2.iter().map(|&i| Value::Int64(i)).collect();
        DataChunk::new(vec![
            ValueVector::from_values(&v1),
            ValueVector::from_values(&v2),
        ])
    }

    #[test]
    fn test_global_count() {
        let mut agg = AggregatePushOperator::global(vec![AggregateExpr::count_star()]);
        let mut sink = CollectorSink::new();

        agg.push(create_test_chunk(&[1, 2, 3, 4, 5]), &mut sink)
            .unwrap();
        agg.finalize(&mut sink).unwrap();

        let chunks = sink.into_chunks();
        assert_eq!(chunks.len(), 1);
        assert_eq!(
            chunks[0].column(0).unwrap().get_value(0),
            Some(Value::Int64(5))
        );
    }

    #[test]
    fn test_global_sum() {
        let mut agg = AggregatePushOperator::global(vec![AggregateExpr::sum(0)]);
        let mut sink = CollectorSink::new();

        agg.push(create_test_chunk(&[1, 2, 3, 4, 5]), &mut sink)
            .unwrap();
        agg.finalize(&mut sink).unwrap();

        let chunks = sink.into_chunks();
        assert_eq!(
            chunks[0].column(0).unwrap().get_value(0),
            Some(Value::Float64(15.0))
        );
    }

    #[test]
    fn test_global_min_max() {
        let mut agg =
            AggregatePushOperator::global(vec![AggregateExpr::min(0), AggregateExpr::max(0)]);
        let mut sink = CollectorSink::new();

        agg.push(create_test_chunk(&[3, 1, 4, 1, 5, 9, 2, 6]), &mut sink)
            .unwrap();
        agg.finalize(&mut sink).unwrap();

        let chunks = sink.into_chunks();
        assert_eq!(
            chunks[0].column(0).unwrap().get_value(0),
            Some(Value::Int64(1))
        );
        assert_eq!(
            chunks[0].column(1).unwrap().get_value(0),
            Some(Value::Int64(9))
        );
    }

    #[test]
    fn test_group_by_sum() {
        // Group by column 0, sum column 1
        let mut agg = AggregatePushOperator::new(vec![0], vec![AggregateExpr::sum(1)]);
        let mut sink = CollectorSink::new();

        // Group 1: 10, 20 (sum=30), Group 2: 30, 40 (sum=70)
        agg.push(
            create_two_column_chunk(&[1, 1, 2, 2], &[10, 20, 30, 40]),
            &mut sink,
        )
        .unwrap();
        agg.finalize(&mut sink).unwrap();

        let chunks = sink.into_chunks();
        assert_eq!(chunks[0].len(), 2); // 2 groups
    }

    #[test]
    #[cfg(feature = "spill")]
    fn test_spillable_aggregate_no_spill() {
        // When threshold is not reached, should work like normal aggregate
        let mut agg = SpillableAggregatePushOperator::new(vec![0], vec![AggregateExpr::sum(1)])
            .with_threshold(100);
        let mut sink = CollectorSink::new();

        agg.push(
            create_two_column_chunk(&[1, 1, 2, 2], &[10, 20, 30, 40]),
            &mut sink,
        )
        .unwrap();
        agg.finalize(&mut sink).unwrap();

        let chunks = sink.into_chunks();
        assert_eq!(chunks[0].len(), 2); // 2 groups
    }

    #[test]
    #[cfg(feature = "spill")]
    fn test_spillable_aggregate_with_spilling() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let manager = Arc::new(SpillManager::new(temp_dir.path()).unwrap());

        // Set very low threshold to force spilling
        let mut agg = SpillableAggregatePushOperator::with_spilling(
            vec![0],
            vec![AggregateExpr::sum(1)],
            manager,
            3, // Spill after 3 groups
        );
        let mut sink = CollectorSink::new();

        // Create 10 different groups
        for i in 0..10 {
            let chunk = create_two_column_chunk(&[i], &[i * 10]);
            agg.push(chunk, &mut sink).unwrap();
        }
        agg.finalize(&mut sink).unwrap();

        let chunks = sink.into_chunks();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].len(), 10); // 10 groups

        // Verify sums are correct
        let mut sums: Vec<f64> = Vec::new();
        for i in 0..chunks[0].len() {
            if let Some(Value::Float64(sum)) = chunks[0].column(1).unwrap().get_value(i) {
                sums.push(sum);
            }
        }
        sums.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(
            sums,
            vec![0.0, 10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0, 80.0, 90.0]
        );
    }

    #[test]
    #[cfg(feature = "spill")]
    fn test_spillable_aggregate_global() {
        // Global aggregation shouldn't be affected by spilling
        let mut agg = SpillableAggregatePushOperator::global(vec![AggregateExpr::count_star()]);
        let mut sink = CollectorSink::new();

        agg.push(create_test_chunk(&[1, 2, 3, 4, 5]), &mut sink)
            .unwrap();
        agg.finalize(&mut sink).unwrap();

        let chunks = sink.into_chunks();
        assert_eq!(chunks.len(), 1);
        assert_eq!(
            chunks[0].column(0).unwrap().get_value(0),
            Some(Value::Int64(5))
        );
    }

    #[test]
    #[cfg(feature = "spill")]
    fn test_spillable_aggregate_many_groups() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let manager = Arc::new(SpillManager::new(temp_dir.path()).unwrap());

        let mut agg = SpillableAggregatePushOperator::with_spilling(
            vec![0],
            vec![AggregateExpr::count_star()],
            manager,
            10, // Very low threshold
        );
        let mut sink = CollectorSink::new();

        // Create 100 different groups
        for i in 0..100 {
            let chunk = create_test_chunk(&[i]);
            agg.push(chunk, &mut sink).unwrap();
        }
        agg.finalize(&mut sink).unwrap();

        let chunks = sink.into_chunks();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].len(), 100); // 100 groups

        // Each group should have count = 1
        for i in 0..100 {
            if let Some(Value::Int64(count)) = chunks[0].column(1).unwrap().get_value(i) {
                assert_eq!(count, 1);
            }
        }
    }

    // ---------------------------------------------------------------
    // hash_value coverage for all Value variants
    // ---------------------------------------------------------------

    #[test]
    fn hash_value_null() {
        let h = hash_value(&Value::Null);
        assert_ne!(h, 0); // hasher produces non-zero for Null discriminant
    }

    #[test]
    fn hash_value_bool() {
        let t = hash_value(&Value::Bool(true));
        let f = hash_value(&Value::Bool(false));
        assert_ne!(t, f);
    }

    #[test]
    fn hash_value_int64() {
        let a = hash_value(&Value::Int64(42));
        let b = hash_value(&Value::Int64(43));
        assert_ne!(a, b);
    }

    #[test]
    fn hash_value_float64() {
        let a = hash_value(&Value::Float64(19.88));
        let b = hash_value(&Value::Float64(3.19));
        assert_ne!(a, b);
    }

    #[test]
    fn hash_value_string() {
        let a = hash_value(&Value::String("hello".into()));
        let b = hash_value(&Value::String("world".into()));
        assert_ne!(a, b);
    }

    #[test]
    fn hash_value_bytes() {
        let a = hash_value(&Value::Bytes(vec![1, 2, 3].into()));
        let b = hash_value(&Value::Bytes(vec![4, 5, 6].into()));
        assert_ne!(a, b);
    }

    #[test]
    fn hash_value_list() {
        let a = hash_value(&Value::List(vec![Value::Int64(1), Value::Int64(2)].into()));
        let b = hash_value(&Value::List(vec![Value::Int64(3)].into()));
        assert_ne!(a, b);
    }

    #[test]
    fn hash_value_map() {
        use grafeo_common::types::PropertyKey;
        use std::collections::BTreeMap;
        use std::sync::Arc;
        let mut map = BTreeMap::new();
        map.insert(PropertyKey::new("key"), Value::Int64(42));
        let h = hash_value(&Value::Map(Arc::new(map)));
        assert_ne!(h, 0);
    }

    #[test]
    fn hash_value_vector() {
        let h = hash_value(&Value::Vector(vec![1.0, 2.0, 3.0].into()));
        assert_ne!(h, 0);
    }

    #[test]
    fn hash_value_path() {
        let h = hash_value(&Value::Path {
            nodes: vec![Value::Int64(1), Value::Int64(2)].into(),
            edges: vec![Value::Int64(10)].into(),
        });
        assert_ne!(h, 0);
    }

    #[test]
    fn hash_value_gcounter() {
        use std::sync::Arc;
        let mut map = std::collections::HashMap::new();
        map.insert("replica1".to_string(), 10u64);
        let h = hash_value(&Value::GCounter(Arc::new(map)));
        assert_ne!(h, 0);
    }

    #[test]
    fn hash_value_on_counter() {
        use std::sync::Arc;
        let mut pos = std::collections::HashMap::new();
        pos.insert("replica1".to_string(), 10u64);
        let neg = std::collections::HashMap::new();
        let h = hash_value(&Value::OnCounter {
            pos: Arc::new(pos),
            neg: Arc::new(neg),
        });
        assert_ne!(h, 0);
    }

    #[test]
    fn hash_value_timestamp() {
        use grafeo_common::types::Timestamp;
        let h = hash_value(&Value::Timestamp(Timestamp::from_micros(1_700_000_000_000)));
        assert_ne!(h, 0);
    }

    #[test]
    fn hash_value_date() {
        use grafeo_common::types::Date;
        let h = hash_value(&Value::Date(Date::from_days(19000)));
        assert_ne!(h, 0);
    }

    #[test]
    fn hash_value_time() {
        use grafeo_common::types::Time;
        let h = hash_value(&Value::Time(Time::from_hms(12, 0, 0).unwrap()));
        assert_ne!(h, 0);
    }

    #[test]
    fn hash_value_duration() {
        use grafeo_common::types::Duration;
        let h = hash_value(&Value::Duration(Duration::from_days(1)));
        assert_ne!(h, 0);
    }

    #[test]
    fn hash_value_zoned_datetime() {
        use grafeo_common::types::{Timestamp, ZonedDatetime};
        let zdt =
            ZonedDatetime::from_timestamp_offset(Timestamp::from_micros(1_700_000_000_000), 3600);
        let h = hash_value(&Value::ZonedDatetime(zdt));
        assert_ne!(h, 0);
    }

    // ---------------------------------------------------------------
    // Accumulator finalize for advanced functions (fallback to Null)
    // ---------------------------------------------------------------

    #[test]
    fn finalize_advanced_functions_return_null() {
        let advanced = [
            AggregateFunction::Last,
            AggregateFunction::Collect,
            AggregateFunction::StdDev,
            AggregateFunction::StdDevPop,
            AggregateFunction::Variance,
            AggregateFunction::VariancePop,
            AggregateFunction::PercentileDisc,
            AggregateFunction::PercentileCont,
            AggregateFunction::GroupConcat,
            AggregateFunction::Sample,
            AggregateFunction::CovarSamp,
            AggregateFunction::CovarPop,
            AggregateFunction::Corr,
            AggregateFunction::RegrSlope,
            AggregateFunction::RegrIntercept,
            AggregateFunction::RegrR2,
            AggregateFunction::RegrCount,
            AggregateFunction::RegrSxx,
            AggregateFunction::RegrSyy,
            AggregateFunction::RegrSxy,
            AggregateFunction::RegrAvgx,
            AggregateFunction::RegrAvgy,
        ];

        for func in advanced {
            let mut acc = Accumulator::new();
            acc.add(&Value::Int64(42));
            let result = acc.finalize(func);
            assert_eq!(
                result,
                Value::Null,
                "Advanced function {func:?} should return Null in push accumulator"
            );
        }
    }

    #[test]
    fn finalize_first_returns_first_value() {
        let mut acc = Accumulator::new();
        acc.add(&Value::Int64(10));
        acc.add(&Value::Int64(20));
        assert_eq!(acc.finalize(AggregateFunction::First), Value::Int64(10));
    }

    #[test]
    fn finalize_avg_empty_returns_null() {
        let mut acc = Accumulator::new();
        assert_eq!(acc.finalize(AggregateFunction::Avg), Value::Null);
    }

    #[test]
    fn finalize_sum_empty_returns_null() {
        let mut acc = Accumulator::new();
        assert_eq!(acc.finalize(AggregateFunction::Sum), Value::Null);
    }

    #[test]
    fn finalize_min_max_empty_returns_null() {
        let mut acc_min = Accumulator::new();
        let mut acc_max = Accumulator::new();
        assert_eq!(acc_min.finalize(AggregateFunction::Min), Value::Null);
        assert_eq!(acc_max.finalize(AggregateFunction::Max), Value::Null);
    }

    #[test]
    fn accumulator_skips_nulls() {
        let mut acc = Accumulator::new();
        acc.add(&Value::Null);
        acc.add(&Value::Int64(5));
        acc.add(&Value::Null);
        assert_eq!(acc.count, 1);
        assert_eq!(acc.finalize(AggregateFunction::Count), Value::Int64(1));
    }

    #[test]
    fn test_empty_chunk_returns_ok() {
        let mut agg = AggregatePushOperator::global(vec![AggregateExpr::count_star()]);
        let mut sink = CollectorSink::new();
        let empty = DataChunk::new(vec![ValueVector::new()]);
        let result = agg.push(empty, &mut sink).unwrap();
        assert!(result);
    }
}
