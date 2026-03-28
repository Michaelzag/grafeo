/**
 * Vitest spec runner for .gtest files.
 *
 * Discovers all .gtest files under tests/spec/, parses them, and creates
 * vitest tests that execute queries through the Node.js GrafeoDB bindings.
 */

import { describe, it, expect } from 'vitest'
import { readFileSync, readdirSync, statSync, existsSync } from 'fs'
import { join, relative, resolve } from 'path'
import { parseGtestFile } from './parser.mjs'
import { assertRowsSorted, assertRowsOrdered, assertRowsWithPrecision, assertHash, resultToRows } from './comparator.mjs'

// ---------------------------------------------------------------------------
// Import GrafeoDB (skip all tests gracefully if unavailable)
// ---------------------------------------------------------------------------

let GrafeoDB
let GRAFEO_AVAILABLE = false

try {
  const mod = await import('../../../../crates/bindings/node/index.js')
  GrafeoDB = mod.GrafeoDB
  GRAFEO_AVAILABLE = true
} catch {
  // Bindings not built; all tests will be skipped
}

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

const SPEC_DIR = resolve(import.meta.dirname, '..', '..')
const DATASETS_DIR = join(SPEC_DIR, 'datasets')

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
async function loadDataset(db, datasetName) {
  const setupPath = join(DATASETS_DIR, `${datasetName}.setup`)
  if (!existsSync(setupPath)) {
    throw new Error(`Dataset file not found: ${setupPath}`)
  }
  const content = readFileSync(setupPath, 'utf-8')
  for (const line of content.split(/\r?\n/)) {
    const trimmed = line.trim()
    if (!trimmed || trimmed.startsWith('#')) continue
    await db.execute(trimmed)
  }
}

/** Execute a query in the specified language. */
async function executeQuery(db, language, query) {
  switch (language) {
    case 'gql':
    case '':
      return db.execute(query)
    case 'cypher':
      if (!db.executeCypher) throw new Error('Cypher not available')
      return db.executeCypher(query)
    case 'gremlin':
      if (!db.executeGremlin) throw new Error('Gremlin not available')
      return db.executeGremlin(query)
    case 'graphql':
      if (!db.executeGraphql) throw new Error('GraphQL not available')
      return db.executeGraphql(query)
    case 'sparql':
      if (!db.executeSparql) throw new Error('SPARQL not available')
      return db.executeSparql(query)
    case 'sql-pgq':
    case 'sql_pgq':
      if (!db.executeSql) throw new Error('SQL/PGQ not available')
      return db.executeSql(query)
    default:
      throw new Error(`Unsupported language: ${language}`)
  }
}

/** Check if a language method is available on the GrafeoDB instance. */
function isLanguageAvailable(db, language) {
  switch (language) {
    case 'gql': case '': return true
    case 'cypher': return typeof db.executeCypher === 'function'
    case 'gremlin': return typeof db.executeGremlin === 'function'
    case 'graphql': return typeof db.executeGraphql === 'function'
    case 'sparql': return typeof db.executeSparql === 'function'
    case 'rdf': return typeof db.executeSparql === 'function'
    case 'sql-pgq': case 'sql_pgq': return typeof db.executeSql === 'function'
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
          it(`${tc.name}_${lang}`, async (ctx) => {
            if (!GRAFEO_AVAILABLE) return ctx.skip()
            const db = GrafeoDB.create()
            try {
              if (!isLanguageAvailable(db, lang)) return ctx.skip()
              if (meta.dataset && meta.dataset !== 'empty') {
                await loadDataset(db, meta.dataset)
              }
              await runTestCase(db, { ...tc, query }, lang, meta.language || 'gql')
            } finally {
              db.close()
            }
          })
        }
        continue
      }

      it(tc.name, async (ctx) => {
        if (!GRAFEO_AVAILABLE) return ctx.skip()

        // Skip by field
        if (tc.skip) return ctx.skip()

        const db = GrafeoDB.create()
        try {
          // Check language availability
          if (!isLanguageAvailable(db, meta.language)) return ctx.skip()

          // Check requires: skip if binding does not expose the required method
          for (const req of meta.requires) {
            if (!isLanguageAvailable(db, req)) return ctx.skip()
          }

          // Load dataset
          if (meta.dataset && meta.dataset !== 'empty') {
            await loadDataset(db, meta.dataset)
          }

          await runTestCase(db, tc, meta.language, meta.language || 'gql')
        } finally {
          db.close()
        }
      })
    }
  })
}

/** Execute a single test case and assert the expected result. */
async function runTestCase(db, tc, language, setupLanguage) {
  // Run setup queries in the file's declared language (not the variant language)
  for (const setupQ of tc.setup) {
    await executeQuery(db, setupLanguage || language, setupQ)
  }

  const exp = tc.expect

  // Determine queries
  const queries = tc.statements.length > 0 ? tc.statements : tc.query ? [tc.query] : []
  if (queries.length === 0) throw new Error(`No query or statements in test '${tc.name}'`)

  // Error case: execute all-but-last normally, only last should fail
  if (exp.error) {
    for (let i = 0; i < queries.length - 1; i++) {
      await executeQuery(db, language, queries[i])
    }
    try {
      await executeQuery(db, language, queries[queries.length - 1])
      throw new Error(`Expected error containing '${exp.error}' but query succeeded`)
    } catch (err) {
      if (err.message.startsWith('Expected error')) throw err
      expect(err.message || String(err)).toContain(exp.error)
    }
    return
  }

  // Execute all queries, capture last result
  let result
  for (let i = 0; i < queries.length; i++) {
    result = await executeQuery(db, language, queries[i])
  }

  // Column assertion (checked before value assertions)
  if (exp.columns && exp.columns.length > 0) {
    const actualCols = result.columns ? [...result.columns] : []
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

  // Hash check
  if (exp.hash) {
    assertHash(result, exp.hash)
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
