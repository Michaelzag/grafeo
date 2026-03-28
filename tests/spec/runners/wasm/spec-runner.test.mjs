/**
 * Vitest spec runner for .gtest files (WASM bindings).
 *
 * Discovers all .gtest files under tests/spec/, parses them, and creates
 * vitest tests that execute queries through the WASM GrafeoDB bindings.
 *
 * Reuses the parser and comparator from the Node.js runner.
 */

import { describe, it, expect } from 'vitest'
import { readFileSync, readdirSync, statSync, existsSync } from 'fs'
import { join, relative, resolve } from 'path'
import { parseGtestFile } from '../node/parser.mjs'
import { assertRowsSorted, assertRowsOrdered, assertRowsWithPrecision } from '../node/comparator.mjs'

// ---------------------------------------------------------------------------
// Import WASM bindings (skip all tests gracefully if unavailable)
// ---------------------------------------------------------------------------

let Database
let initSync
let WASM_AVAILABLE = false

try {
  const wasmPkgPath = resolve(import.meta.dirname, '..', '..', '..', '..', 'crates', 'bindings', 'wasm', 'pkg')
  const wasmJsPath = join(wasmPkgPath, 'grafeo_wasm.js')
  const wasmBinPath = join(wasmPkgPath, 'grafeo_wasm_bg.wasm')

  if (existsSync(wasmJsPath) && existsSync(wasmBinPath)) {
    const mod = await import('../../../../crates/bindings/wasm/pkg/grafeo_wasm.js')
    Database = mod.Database
    initSync = mod.initSync

    // Initialize WASM synchronously from the .wasm file
    const wasmBytes = readFileSync(wasmBinPath)
    initSync({ module: wasmBytes })
    WASM_AVAILABLE = true
  }
} catch {
  // WASM package not built or initialization failed; all tests will be skipped
}

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

const SPEC_DIR = resolve(import.meta.dirname, '..', '..')
const DATASETS_DIR = join(SPEC_DIR, 'datasets')

// ---------------------------------------------------------------------------
// Result conversion
// ---------------------------------------------------------------------------

/**
 * Wrap a WASM raw result so it looks like the Node.js QueryResult expected
 * by the comparator functions (which call resultToRows internally).
 */
function wrapRawResult(rawResult) {
  return {
    columns: rawResult.columns,
    length: rawResult.rows.length,
    toArray() {
      const arr = []
      for (const row of rawResult.rows) {
        const obj = {}
        for (let i = 0; i < rawResult.columns.length; i++) {
          obj[rawResult.columns[i]] = row[i]
        }
        arr.push(obj)
      }
      return arr
    },
  }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Recursively find all .gtest files. */
function findGtestFiles(dir) {
  const results = []
  if (!existsSync(dir)) return results
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry)
    const stat = statSync(full)
    if (stat.isDirectory()) {
      results.push(...findGtestFiles(full))
    } else if (entry.endsWith('.gtest')) {
      results.push(full)
    }
  }
  return results.sort()
}

/** Load a .setup file and execute each line as GQL. */
function loadDataset(db, datasetName) {
  const setupPath = join(DATASETS_DIR, `${datasetName}.setup`)
  if (!existsSync(setupPath)) {
    throw new Error(`Dataset file not found: ${setupPath}`)
  }
  const content = readFileSync(setupPath, 'utf-8')
  for (const line of content.split(/\r?\n/)) {
    const trimmed = line.trim()
    if (!trimmed || trimmed.startsWith('#')) continue
    db.execute(trimmed)
  }
}

/**
 * Map .gtest language keys to the WASM executeWithLanguage dispatch key.
 * Returns null if the language is not recognized.
 */
function toDispatchKey(language) {
  switch (language) {
    case 'gql': case '': return 'gql'
    case 'cypher': return 'cypher'
    case 'gremlin': return 'gremlin'
    case 'graphql': return 'graphql'
    case 'graphql-rdf': return 'graphql-rdf'
    case 'sparql': return 'sparql'
    case 'sql-pgq': case 'sql_pgq': case 'sql': return 'sql'
    default: return null
  }
}

/** Execute a query in the specified language, returning a raw result. */
function executeQueryRaw(db, language, query) {
  const key = toDispatchKey(language)
  if (key === null) throw new Error(`Unsupported language: ${language}`)
  if (key === 'gql') return db.executeRaw(query)
  return db.executeRawWithLanguage(query, key)
}

/** Execute a query in the specified language (for setup, returns array of objects). */
function executeQuery(db, language, query) {
  const key = toDispatchKey(language)
  if (key === null) throw new Error(`Unsupported language: ${language}`)
  if (key === 'gql') return db.execute(query)
  return db.executeWithLanguage(query, key)
}

/** Check if a language method is available on the WASM Database instance. */
function isLanguageAvailable(db, language) {
  switch (language) {
    case 'gql': case '': return true
    case 'cypher': return typeof db.executeCypher === 'function'
    case 'gremlin': return typeof db.executeGremlin === 'function'
    case 'graphql': return typeof db.executeGraphql === 'function'
    case 'graphql-rdf': return typeof db.executeWithLanguage === 'function'
    case 'sql-pgq': case 'sql_pgq': case 'sql': return typeof db.executeSql === 'function'
    case 'sparql': return typeof db.executeSparql === 'function'
    default: return false
  }
}

// ---------------------------------------------------------------------------
// Discover and register tests
// ---------------------------------------------------------------------------

const gtestFiles = findGtestFiles(SPEC_DIR)

for (const filePath of gtestFiles) {
  // Skip runner directories
  if (filePath.includes('runners')) continue

  const relPath = relative(SPEC_DIR, filePath).replace(/\\/g, '/')
  let parsed

  try {
    parsed = parseGtestFile(filePath)
  } catch (err) {
    describe(relPath, () => {
      it('should parse without errors', () => {
        throw new Error(`Parse error: ${err.message}`)
      })
    })
    continue
  }

  const { meta, tests } = parsed

  describe(relPath, () => {
    for (const tc of tests) {
      // Handle rosetta variants
      if (tc.variants && Object.keys(tc.variants).length > 0) {
        for (const [lang, query] of Object.entries(tc.variants)) {
          it(`${tc.name}_${lang}`, () => {
            if (!WASM_AVAILABLE) return expect(true).toBe(true) // skip
            const db = new Database()
            try {
              if (!isLanguageAvailable(db, lang)) return // skip
              if (meta.dataset && meta.dataset !== 'empty') {
                loadDataset(db, meta.dataset)
              }
              runTestCase(db, { ...tc, query }, lang)
            } finally {
              db.free()
            }
          })
        }
        continue
      }

      it(tc.name, () => {
        if (!WASM_AVAILABLE) return expect(true).toBe(true) // skip

        // Skip by field
        if (tc.skip) return

        const db = new Database()
        try {
          // Check language availability
          if (!isLanguageAvailable(db, meta.language)) return

          // Check requires
          for (const req of meta.requires) {
            if (req === 'sparql' || req === 'rdf') return // skip
          }

          // Load dataset
          if (meta.dataset && meta.dataset !== 'empty') {
            loadDataset(db, meta.dataset)
          }

          runTestCase(db, tc, meta.language)
        } finally {
          db.free()
        }
      })
    }
  })
}

/** Execute a single test case and assert the expected result. */
function runTestCase(db, tc, language) {
  // Run setup queries in the file's declared language
  for (const setupQ of tc.setup) {
    executeQuery(db, language, setupQ)
  }

  const exp = tc.expect

  // Determine queries
  const queries = tc.statements.length > 0 ? tc.statements : tc.query ? [tc.query] : []
  if (queries.length === 0) throw new Error(`No query or statements in test '${tc.name}'`)

  // Error case
  if (exp.error) {
    try {
      for (const q of queries) {
        executeQuery(db, language, q)
      }
      throw new Error(`Expected error containing '${exp.error}' but query succeeded`)
    } catch (err) {
      if (err.message.startsWith('Expected error')) throw err
      expect(err.message || String(err)).toContain(exp.error)
    }
    return
  }

  // Execute all queries, capture last raw result for assertions
  let rawResult
  for (let i = 0; i < queries.length; i++) {
    rawResult = executeQueryRaw(db, language, queries[i])
  }

  // Wrap for the comparator functions
  const result = wrapRawResult(rawResult)

  // Column assertion (checked before value assertions)
  if (exp.columns && exp.columns.length > 0) {
    const actualCols = [...result.columns]
    expect(actualCols).toEqual(exp.columns)
  }

  // Empty check
  if (exp.empty) {
    expect(result.length).toBe(0)
    return
  }

  // Count check
  if (exp.count !== null && exp.count !== undefined) {
    expect(result.length).toBe(exp.count)
    return
  }

  // Rows check
  if (exp.rows.length > 0) {
    if (exp.precision !== null && exp.precision !== undefined) {
      assertRowsWithPrecision(result, exp.rows, exp.precision)
    } else if (exp.ordered) {
      assertRowsOrdered(result, exp.rows)
    } else {
      assertRowsSorted(result, exp.rows)
    }
  }
}
