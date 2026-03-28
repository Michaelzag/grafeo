/**
 * Result comparison logic for .gtest spec tests.
 *
 * Mirrors the assertion helpers in grafeo-spec-tests/src/lib.rs so that
 * the Node.js runner validates results identically to the Rust runner.
 */

/**
 * Convert a JS value to its canonical string for comparison.
 * Must match Rust's value_to_string in lib.rs.
 */
export function valueToString(val) {
  if (val === null || val === undefined) return 'null'
  if (typeof val === 'boolean') return val ? 'true' : 'false'
  if (typeof val === 'bigint') return val.toString()
  if (typeof val === 'number') {
    if (!isFinite(val)) return val > 0 ? 'Infinity' : '-Infinity'
    if (isNaN(val)) return 'NaN'
    // Rust: format!("{}", 15.0_f64) -> "15" (no trailing .0)
    if (Number.isInteger(val)) return val.toString()
    return val.toString()
  }
  if (Array.isArray(val)) {
    const inner = val.map(valueToString).join(', ')
    return `[${inner}]`
  }
  if (typeof val === 'object' && val !== null) {
    // Date-like objects
    if (val instanceof Date) return val.toISOString()
    // Plain object (map)
    const entries = Object.entries(val)
      .map(([k, v]) => `${k}: ${valueToString(v)}`)
      .sort()
    return `{${entries.join(', ')}}`
  }
  return String(val)
}

/**
 * Convert a GrafeoDB QueryResult to rows of canonical strings.
 * @param {object} result - QueryResult from db.execute()
 * @returns {string[][]}
 */
export function resultToRows(result) {
  const columns = result.columns
  const rows = []
  const arr = result.toArray()
  for (const row of arr) {
    const r = []
    for (const col of columns) {
      r.push(valueToString(row[col]))
    }
    rows.push(r)
  }
  return rows
}

/**
 * Assert rows match after sorting both sides.
 */
export function assertRowsSorted(result, expected) {
  const actual = resultToRows(result)
  const sortedActual = [...actual].sort((a, b) => a.join('|').localeCompare(b.join('|')))
  const sortedExpected = [...expected].sort((a, b) => a.join('|').localeCompare(b.join('|')))

  if (sortedActual.length !== sortedExpected.length) {
    throw new Error(
      `Row count mismatch: got ${sortedActual.length}, expected ${sortedExpected.length}\n` +
      `Actual: ${JSON.stringify(sortedActual)}\nExpected: ${JSON.stringify(sortedExpected)}`
    )
  }
  for (let i = 0; i < sortedActual.length; i++) {
    for (let j = 0; j < sortedActual[i].length; j++) {
      if (sortedActual[i][j] !== sortedExpected[i][j]) {
        throw new Error(
          `Mismatch at sorted row ${i}, col ${j}: got '${sortedActual[i][j]}', expected '${sortedExpected[i][j]}'\n` +
          `Actual row: ${JSON.stringify(sortedActual[i])}\nExpected row: ${JSON.stringify(sortedExpected[i])}`
        )
      }
    }
  }
}

/**
 * Assert rows match in exact order.
 */
export function assertRowsOrdered(result, expected) {
  const actual = resultToRows(result)
  if (actual.length !== expected.length) {
    throw new Error(
      `Row count mismatch: got ${actual.length}, expected ${expected.length}\n` +
      `Actual: ${JSON.stringify(actual)}\nExpected: ${JSON.stringify(expected)}`
    )
  }
  for (let i = 0; i < actual.length; i++) {
    for (let j = 0; j < actual[i].length; j++) {
      if (actual[i][j] !== expected[i][j]) {
        throw new Error(
          `Mismatch at row ${i}, col ${j}: got '${actual[i][j]}', expected '${expected[i][j]}'\n` +
          `Actual row: ${JSON.stringify(actual[i])}\nExpected row: ${JSON.stringify(expected[i])}`
        )
      }
    }
  }
}
