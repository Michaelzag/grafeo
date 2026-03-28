// Package grafeo_test discovers and executes .gtest spec files through the Go
// grafeo bindings. Each .gtest file becomes a top-level subtest, and each test
// case within becomes a nested subtest (Go table-driven test pattern).
package grafeo_test

import (
	"bufio"
	"crypto/md5"
	"fmt"
	"math"
	"os"
	"path/filepath"
	"runtime"
	"sort"
	"strconv"
	"strings"
	"testing"

	grafeo "github.com/GrafeoDB/grafeo/crates/bindings/go"
	"gopkg.in/yaml.v3"
)

// ---------------------------------------------------------------------------
// .gtest schema types
// ---------------------------------------------------------------------------

// GtestFile is the top-level structure of a .gtest YAML file.
type GtestFile struct {
	Meta  Meta       `yaml:"meta"`
	Tests []TestCase `yaml:"tests"`
}

// Meta holds file-level metadata such as the query language and dataset.
type Meta struct {
	Language string   `yaml:"language"`
	Model    string   `yaml:"model"`
	Section  string   `yaml:"section"`
	Title    string   `yaml:"title"`
	Dataset  string   `yaml:"dataset"`
	Requires []string `yaml:"requires"`
	Tags     []string `yaml:"tags"`
}

// TestCase represents a single test within a .gtest file.
type TestCase struct {
	Name       string            `yaml:"name"`
	Query      string            `yaml:"query"`
	Statements []string          `yaml:"statements"`
	Setup      []string          `yaml:"setup"`
	Params     map[string]string `yaml:"params"`
	Tags       []string          `yaml:"tags"`
	Skip       string            `yaml:"skip"`
	Expect     Expect            `yaml:"expect"`
	Variants   map[string]string `yaml:"variants"`
}

// Expect holds the expected result assertions for a test case.
type Expect struct {
	Rows      [][]string `yaml:"-"` // Populated by custom UnmarshalYAML
	Ordered   bool       `yaml:"ordered"`
	Count     *int       `yaml:"count"`
	Empty     bool       `yaml:"empty"`
	Error     *string    `yaml:"error"`
	Hash      *string    `yaml:"hash"`
	Precision *int       `yaml:"precision"`
	Columns   []string   `yaml:"columns"`
}

// UnmarshalYAML handles the Expect block, manually parsing `rows` to preserve
// null values that Go's default YAML decoder would skip in slices.
func (e *Expect) UnmarshalYAML(node *yaml.Node) error {
	if node.Kind != yaml.MappingNode {
		return nil
	}
	for i := 0; i+1 < len(node.Content); i += 2 {
		key := node.Content[i].Value
		val := node.Content[i+1]
		switch key {
		case "rows":
			if val.Kind == yaml.SequenceNode {
				for _, rowNode := range val.Content {
					if rowNode.Kind == yaml.SequenceNode {
						cells := make([]string, len(rowNode.Content))
						for j, cell := range rowNode.Content {
							cells[j] = nodeToCanonical(cell)
						}
						e.Rows = append(e.Rows, cells)
					}
				}
			}
		case "ordered":
			e.Ordered = val.Value == "true"
		case "count":
			if v, err := strconv.Atoi(val.Value); err == nil {
				e.Count = &v
			}
		case "empty":
			e.Empty = val.Value == "true"
		case "error":
			s := val.Value
			e.Error = &s
		case "hash":
			s := val.Value
			e.Hash = &s
		case "precision":
			if v, err := strconv.Atoi(val.Value); err == nil {
				e.Precision = &v
			}
		case "columns":
			if val.Kind == yaml.SequenceNode {
				for _, c := range val.Content {
					e.Columns = append(e.Columns, c.Value)
				}
			}
		}
	}
	return nil
}

// ---------------------------------------------------------------------------
// Language dispatch
// ---------------------------------------------------------------------------

// dispatchKey maps the language name from a .gtest file to the string accepted
// by db.ExecuteLanguage (which ultimately calls the C FFI).
func dispatchKey(lang string) string {
	switch lang {
	case "sql-pgq", "sql_pgq":
		return "sql"
	case "":
		return "gql"
	default:
		return lang
	}
}

// executeQuery runs a query in the specified language.
func executeQuery(db *grafeo.Database, language, query string) (*grafeo.QueryResult, error) {
	key := dispatchKey(language)
	if key == "gql" {
		return db.Execute(query)
	}
	return db.ExecuteLanguage(key, query, "")
}

// ---------------------------------------------------------------------------
// Canonical value serialization
// ---------------------------------------------------------------------------

// nodeToCanonical converts a yaml.Node to the canonical string representation
// that matches the Rust spec-test runner's value_to_string.
func nodeToCanonical(node *yaml.Node) string {
	switch node.Kind {
	case yaml.ScalarNode:
		return scalarToCanonical(node)
	case yaml.SequenceNode:
		parts := make([]string, len(node.Content))
		for i, child := range node.Content {
			parts[i] = nodeToCanonical(child)
		}
		return "[" + strings.Join(parts, ", ") + "]"
	case yaml.MappingNode:
		entries := make([]string, 0, len(node.Content)/2)
		for i := 0; i+1 < len(node.Content); i += 2 {
			k := node.Content[i].Value
			v := nodeToCanonical(node.Content[i+1])
			entries = append(entries, k+": "+v)
		}
		sort.Strings(entries)
		return "{" + strings.Join(entries, ", ") + "}"
	case yaml.AliasNode:
		return nodeToCanonical(node.Alias)
	default:
		return node.Value
	}
}

// scalarToCanonical converts a YAML scalar node to its canonical string.
func scalarToCanonical(node *yaml.Node) string {
	// Null
	if node.Tag == "!!null" || node.Value == "null" || node.Value == "~" || node.Value == "" {
		if node.Tag == "!!null" || node.Value == "null" || node.Value == "~" {
			return "null"
		}
		// Empty string with explicit tag
		if node.Tag == "!!str" {
			return ""
		}
		return "null"
	}

	// Boolean
	if node.Tag == "!!bool" {
		switch strings.ToLower(node.Value) {
		case "true", "yes", "on":
			return "true"
		case "false", "no", "off":
			return "false"
		}
	}

	// Integer
	if node.Tag == "!!int" {
		return node.Value
	}

	// Float
	if node.Tag == "!!float" {
		switch node.Value {
		case ".nan", ".NaN", ".NAN":
			return "NaN"
		case ".inf", ".Inf", ".INF":
			return "Infinity"
		case "-.inf", "-.Inf", "-.INF":
			return "-Infinity"
		}
		f, err := strconv.ParseFloat(node.Value, 64)
		if err != nil {
			return node.Value
		}
		// Match Rust Display: whole numbers drop trailing .0
		if f == math.Trunc(f) && !math.IsInf(f, 0) && !math.IsNaN(f) && math.Abs(f) < (1<<53) {
			return strconv.FormatInt(int64(f), 10)
		}
		return strconv.FormatFloat(f, 'f', -1, 64)
	}

	// Bare Infinity/NaN strings (yaml.v3 may tag these as !!str)
	switch node.Value {
	case "Infinity", "inf", "+inf":
		return "Infinity"
	case "-Infinity", "-inf":
		return "-Infinity"
	case "NaN":
		return "NaN"
	}

	// String (including quoted scalars)
	return node.Value
}

// valueToString converts a Go interface{} value (as returned by grafeo Row)
// to its canonical string representation for comparison.
func valueToString(v interface{}) string {
	if v == nil {
		return "null"
	}
	switch val := v.(type) {
	case bool:
		if val {
			return "true"
		}
		return "false"
	case float64:
		if math.IsNaN(val) {
			return "NaN"
		}
		if math.IsInf(val, 1) {
			return "Infinity"
		}
		if math.IsInf(val, -1) {
			return "-Infinity"
		}
		// Match Rust Display: whole floats drop .0
		if val == math.Trunc(val) && math.Abs(val) < (1<<53) {
			return strconv.FormatInt(int64(val), 10)
		}
		return strconv.FormatFloat(val, 'f', -1, 64)
	case int:
		return strconv.Itoa(val)
	case int64:
		return strconv.FormatInt(val, 10)
	case string:
		return val
	case []interface{}:
		parts := make([]string, len(val))
		for i, item := range val {
			parts[i] = valueToString(item)
		}
		return "[" + strings.Join(parts, ", ") + "]"
	case map[string]interface{}:
		// Temporal type-tagged values from C FFI JSON: {"$date": "2024-06-15"}
		if len(val) == 1 {
			for k, v := range val {
				switch k {
				case "$date", "$time", "$datetime", "$timestamp", "$zoned_datetime":
					return fmt.Sprintf("%v", v)
				case "$duration":
					return fmt.Sprintf("%v", v)
				}
			}
		}
		// Duration: {months, days, nanos} -> ISO 8601
		if len(val) == 3 {
			if m, ok1 := val["months"]; ok1 {
				if d, ok2 := val["days"]; ok2 {
					if n, ok3 := val["nanos"]; ok3 {
						return durationToISO(int64(toFloat(m)), int64(toFloat(d)), int64(toFloat(n)))
					}
				}
			}
		}
		entries := make([]string, 0, len(val))
		for k, item := range val {
			entries = append(entries, k+": "+valueToString(item))
		}
		sort.Strings(entries)
		return "{" + strings.Join(entries, ", ") + "}"
	default:
		return fmt.Sprintf("%v", val)
	}
}

// toFloat extracts a numeric value from an interface{}.
func toFloat(v interface{}) float64 {
	switch n := v.(type) {
	case float64:
		return n
	case int:
		return float64(n)
	case int64:
		return float64(n)
	default:
		return 0
	}
}

// durationToISO converts {months, days, nanos} to ISO 8601 duration format.
func durationToISO(totalMonths, days, nanos int64) string {
	years := totalMonths / 12
	months := totalMonths % 12
	hours := nanos / 3_600_000_000_000
	rem := nanos % 3_600_000_000_000
	minutes := rem / 60_000_000_000
	rem = rem % 60_000_000_000
	seconds := rem / 1_000_000_000
	subNanos := rem % 1_000_000_000

	var b strings.Builder
	b.WriteString("P")
	if years != 0 {
		fmt.Fprintf(&b, "%dY", years)
	}
	if months != 0 {
		fmt.Fprintf(&b, "%dM", months)
	}
	if days != 0 {
		fmt.Fprintf(&b, "%dD", days)
	}

	var timePart strings.Builder
	if hours != 0 {
		fmt.Fprintf(&timePart, "%dH", hours)
	}
	if minutes != 0 {
		fmt.Fprintf(&timePart, "%dM", minutes)
	}
	if seconds != 0 || subNanos != 0 {
		if subNanos != 0 {
			frac := strings.TrimRight(fmt.Sprintf("%09d", subNanos), "0")
			fmt.Fprintf(&timePart, "%d.%sS", seconds, frac)
		} else {
			fmt.Fprintf(&timePart, "%dS", seconds)
		}
	}
	if timePart.Len() > 0 {
		b.WriteString("T")
		b.WriteString(timePart.String())
	}

	result := b.String()
	if result == "P" {
		return "P0D"
	}
	return result
}

// ---------------------------------------------------------------------------
// Result extraction
// ---------------------------------------------------------------------------

// resultToRows extracts canonical string rows from a QueryResult, in column
// order. If cols is nil, result.Columns is used.
func resultToRows(result *grafeo.QueryResult, cols []string) [][]string {
	if cols == nil {
		cols = result.Columns
	}
	rows := make([][]string, len(result.Rows))
	for i, row := range result.Rows {
		cells := make([]string, len(cols))
		for j, col := range cols {
			cells[j] = valueToString(row[col])
		}
		rows[i] = cells
	}
	return rows
}

// expectedRows returns the already-canonical rows (identity since Rows is [][]string).
func expectedRows(rows [][]string) [][]string {
	out := make([][]string, len(rows))
	for i, row := range rows {
		cells := make([]string, len(row))
		for j, cell := range row {
			cells[j] = cell
		}
		out[i] = cells
	}
	return out
}

// ---------------------------------------------------------------------------
// Assertions
// ---------------------------------------------------------------------------

func assertRowsSorted(t *testing.T, result *grafeo.QueryResult, expected [][]string) {
	t.Helper()
	actual := resultToRows(result, nil)
	sortRows(actual)
	sortRows(expected)
	assertRowsEqual(t, actual, expected, "sorted")
}

func assertRowsOrdered(t *testing.T, result *grafeo.QueryResult, expected [][]string) {
	t.Helper()
	actual := resultToRows(result, nil)
	assertRowsEqual(t, actual, expected, "ordered")
}

func assertRowsEqual(t *testing.T, actual, expected [][]string, mode string) {
	t.Helper()
	if len(actual) != len(expected) {
		t.Fatalf("Row count mismatch (%s): got %d, expected %d\nActual:   %v\nExpected: %v",
			mode, len(actual), len(expected), actual, expected)
	}
	for i := range actual {
		if len(actual[i]) != len(expected[i]) {
			t.Fatalf("Column count mismatch at %s row %d: got %d cols, expected %d\nActual:   %v\nExpected: %v",
				mode, i, len(actual[i]), len(expected[i]), actual[i], expected[i])
		}
		for j := range actual[i] {
			if actual[i][j] != expected[i][j] {
				t.Fatalf("Mismatch at %s row %d, col %d: got %q, expected %q\nFull actual row:   %v\nFull expected row: %v",
					mode, i, j, actual[i][j], expected[i][j], actual[i], expected[i])
			}
		}
	}
}

func assertRowsWithPrecision(t *testing.T, result *grafeo.QueryResult, expected [][]string, precision int) {
	t.Helper()
	actual := resultToRows(result, nil)
	tolerance := math.Pow(10, float64(-precision))

	if len(actual) != len(expected) {
		t.Fatalf("Row count mismatch: got %d, expected %d", len(actual), len(expected))
	}
	for i := range actual {
		if len(actual[i]) != len(expected[i]) {
			t.Fatalf("Column count mismatch at row %d: got %d, expected %d",
				i, len(actual[i]), len(expected[i]))
		}
		for j := range actual[i] {
			af, aErr := strconv.ParseFloat(actual[i][j], 64)
			ef, eErr := strconv.ParseFloat(expected[i][j], 64)
			if aErr == nil && eErr == nil {
				if math.Abs(af-ef) >= tolerance {
					t.Fatalf("Float mismatch at row %d, col %d: got %v, expected %v (tolerance %v)",
						i, j, af, ef, tolerance)
				}
			} else if actual[i][j] != expected[i][j] {
				t.Fatalf("Mismatch at row %d, col %d: got %q, expected %q",
					i, j, actual[i][j], expected[i][j])
			}
		}
	}
}

func assertCount(t *testing.T, result *grafeo.QueryResult, expected int) {
	t.Helper()
	actual := len(result.Rows)
	if actual != expected {
		t.Fatalf("Row count mismatch: got %d, expected %d", actual, expected)
	}
}

func assertEmpty(t *testing.T, result *grafeo.QueryResult) {
	t.Helper()
	actual := len(result.Rows)
	if actual != 0 {
		t.Fatalf("Expected empty result, got %d row(s)", actual)
	}
}

func assertColumns(t *testing.T, result *grafeo.QueryResult, expected []string) {
	t.Helper()
	actual := result.Columns
	if len(actual) != len(expected) {
		t.Fatalf("Column count mismatch: got %v, expected %v", actual, expected)
	}
	for i := range actual {
		if actual[i] != expected[i] {
			t.Fatalf("Column mismatch at index %d: got %q, expected %q\nFull actual:   %v\nFull expected: %v",
				i, actual[i], expected[i], actual, expected)
		}
	}
}

func assertHash(t *testing.T, result *grafeo.QueryResult, expectedHash string) {
	t.Helper()
	rows := resultToRows(result, nil)
	sortRows(rows)

	h := md5.New()
	for _, row := range rows {
		h.Write([]byte(strings.Join(row, "|")))
		h.Write([]byte("\n"))
	}
	actualHash := fmt.Sprintf("%x", h.Sum(nil))
	if actualHash != expectedHash {
		t.Fatalf("Hash mismatch: got %q, expected %q\nRows: %v", actualHash, expectedHash, rows)
	}
}

// sortRows sorts a slice of string slices lexicographically.
func sortRows(rows [][]string) {
	sort.Slice(rows, func(i, j int) bool {
		a, b := rows[i], rows[j]
		for k := 0; k < len(a) && k < len(b); k++ {
			if a[k] != b[k] {
				return a[k] < b[k]
			}
		}
		return len(a) < len(b)
	})
}

// ---------------------------------------------------------------------------
// Dataset loading
// ---------------------------------------------------------------------------

func loadDataset(t *testing.T, db *grafeo.Database, datasetName string, datasetsDir string) {
	t.Helper()
	setupPath := filepath.Join(datasetsDir, datasetName+".setup")
	f, err := os.Open(setupPath)
	if err != nil {
		t.Fatalf("Dataset file not found: %s", setupPath)
	}
	defer f.Close()

	scanner := bufio.NewScanner(f)
	for scanner.Scan() {
		line := strings.TrimSpace(scanner.Text())
		if line == "" || strings.HasPrefix(line, "#") {
			continue
		}
		if _, err := db.Execute(line); err != nil {
			t.Fatalf("Failed to load dataset line %q: %v", line, err)
		}
	}
	if err := scanner.Err(); err != nil {
		t.Fatalf("Error reading dataset %s: %v", setupPath, err)
	}
}

// ---------------------------------------------------------------------------
// Repo root discovery
// ---------------------------------------------------------------------------

// repoRoot walks up from the test file directory until it finds a directory
// containing Cargo.toml (the workspace root).
func repoRoot() string {
	_, file, _, ok := runtime.Caller(0)
	if !ok {
		panic("cannot determine test file location")
	}
	dir := filepath.Dir(file)
	for {
		if _, err := os.Stat(filepath.Join(dir, "Cargo.toml")); err == nil {
			return dir
		}
		parent := filepath.Dir(dir)
		if parent == dir {
			panic("could not find repo root (Cargo.toml) from " + filepath.Dir(file))
		}
		dir = parent
	}
}

// ---------------------------------------------------------------------------
// File discovery
// ---------------------------------------------------------------------------

// findGtestFiles recursively finds all .gtest files under dir, excluding the
// runners subdirectory.
func findGtestFiles(dir string) ([]string, error) {
	var files []string
	err := filepath.Walk(dir, func(path string, info os.FileInfo, err error) error {
		if err != nil {
			return err
		}
		// Skip the runners directory
		if info.IsDir() && info.Name() == "runners" {
			return filepath.SkipDir
		}
		if !info.IsDir() && strings.HasSuffix(info.Name(), ".gtest") {
			files = append(files, path)
		}
		return nil
	})
	if err != nil {
		return nil, err
	}
	sort.Strings(files)
	return files, nil
}

// ---------------------------------------------------------------------------
// .gtest file parsing
// ---------------------------------------------------------------------------

// quoteInlineQueries preprocesses .gtest YAML content to quote inline query
// values that contain {key: value} patterns which confuse yaml.v3's parser.
// Block scalars (query: |) are not affected.
func quoteInlineQueries(data []byte) []byte {
	lines := strings.Split(string(data), "\n")
	for i, line := range lines {
		trimmed := strings.TrimSpace(line)
		// Match "query: <something with braces>" but not "query: |" (block scalar)
		for _, prefix := range []string{"query: ", "- "} {
			if !strings.HasPrefix(trimmed, prefix) {
				continue
			}
			val := trimmed[len(prefix):]
			if val == "" || val == "|" || strings.HasPrefix(val, "\"") || strings.HasPrefix(val, "'") {
				continue
			}
			// If the value contains unquoted {key: value}, quote the entire value
			if strings.Contains(val, "{") && strings.Contains(val, ": ") {
				indent := line[:len(line)-len(strings.TrimLeft(line, " \t"))]
				lines[i] = indent + prefix + "\"" + strings.ReplaceAll(val, "\"", "\\\"") + "\""
			}
		}
	}
	return []byte(strings.Join(lines, "\n"))
}

func parseGtestFile(path string) (*GtestFile, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("reading %s: %w", path, err)
	}
	// Preprocess to handle inline queries with {key: value} patterns
	data = quoteInlineQueries(data)
	var gf GtestFile
	if err := yaml.Unmarshal(data, &gf); err != nil {
		return nil, fmt.Errorf("parsing %s: %w", path, err)
	}
	// Default language to gql
	if gf.Meta.Language == "" {
		gf.Meta.Language = "gql"
	}
	// Default dataset to empty
	if gf.Meta.Dataset == "" {
		gf.Meta.Dataset = "empty"
	}
	return &gf, nil
}

// ---------------------------------------------------------------------------
// Main test function
// ---------------------------------------------------------------------------

func TestSpec(t *testing.T) {
	root := repoRoot()
	specDir := filepath.Join(root, "tests", "spec")
	datasetsDir := filepath.Join(specDir, "datasets")

	gtestFiles, err := findGtestFiles(specDir)
	if err != nil {
		t.Fatalf("Failed to discover .gtest files: %v", err)
	}
	if len(gtestFiles) == 0 {
		t.Fatal("No .gtest files found under tests/spec/")
	}

	for _, filePath := range gtestFiles {
		relPath, _ := filepath.Rel(specDir, filePath)
		relPath = filepath.ToSlash(relPath)

		t.Run(relPath, func(t *testing.T) {
			gf, err := parseGtestFile(filePath)
			if err != nil {
				t.Fatalf("Parse error: %v", err)
			}

			for _, tc := range gf.Tests {
				tc := tc // capture loop variable

				if len(tc.Variants) > 0 {
					// Rosetta: one subtest per variant language
					for lang, query := range tc.Variants {
						lang, query := lang, query // capture
						testName := tc.Name + "[" + lang + "]"
						t.Run(testName, func(t *testing.T) {
							runSingleTest(t, gf, tc, lang, query, datasetsDir)
						})
					}
				} else {
					t.Run(tc.Name, func(t *testing.T) {
						runSingleTest(t, gf, tc, "", "", datasetsDir)
					})
				}
			}
		})
	}
}

// runSingleTest executes a single test case from a .gtest file.
// If variantLang is non-empty, it overrides the file's meta.language and
// variantQuery overrides tc.Query.
func runSingleTest(t *testing.T, gf *GtestFile, tc TestCase, variantLang, variantQuery string, datasetsDir string) {
	t.Helper()

	// Skip if test has a skip field
	if tc.Skip != "" {
		t.Skipf("skipped in .gtest: %s", tc.Skip)
	}

	// Determine language
	language := variantLang
	if language == "" {
		language = gf.Meta.Language
	}

	// Check requires: skip if language is not supported
	// (We cannot introspect available features at the Go level, so we try
	// to execute and skip on specific errors. However, we do skip known
	// unsupported dispatch keys proactively.)
	for _, req := range gf.Meta.Requires {
		if req == "rdf" && language != "sparql" {
			// rdf requirement is only relevant for SPARQL tests
			continue
		}
	}

	// Fresh database per test
	db, err := grafeo.OpenInMemory()
	if err != nil {
		t.Fatalf("Failed to open in-memory database: %v", err)
	}
	defer db.Close()

	// Load dataset
	if gf.Meta.Dataset != "" && gf.Meta.Dataset != "empty" {
		loadDataset(t, db, gf.Meta.Dataset, datasetsDir)
	}

	// Run setup queries in the file's declared language
	setupLang := gf.Meta.Language
	for _, setupQ := range tc.Setup {
		if _, err := executeQuery(db, setupLang, setupQ); err != nil {
			t.Fatalf("Setup query failed: %v\nQuery: %s", err, setupQ)
		}
	}

	// Determine query / statements
	query := variantQuery
	if query == "" {
		query = tc.Query
	}

	var queries []string
	if len(tc.Statements) > 0 {
		queries = tc.Statements
	} else if query != "" {
		queries = []string{query}
	} else {
		t.Fatalf("No query or statements in test %q", tc.Name)
	}

	exp := tc.Expect

	// Error tests
	if exp.Error != nil {
		runErrorTest(t, db, language, queries, *exp.Error)
		return
	}

	// Execute all-but-last as fire-and-forget
	for _, q := range queries[:len(queries)-1] {
		if _, err := executeQuery(db, language, q); err != nil {
			// If the language is not supported, skip rather than fail
			if isUnsupportedLanguageError(err) {
				t.Skipf("Language %q not available: %v", language, err)
			}
			t.Fatalf("Statement failed: %v\nQuery: %s", err, q)
		}
	}

	// Last query: capture result
	result, err := executeQuery(db, language, queries[len(queries)-1])
	if err != nil {
		if isUnsupportedLanguageError(err) {
			t.Skipf("Language %q not available: %v", language, err)
		}
		t.Fatalf("Query failed: %v\nQuery: %s", err, queries[len(queries)-1])
	}

	// Column assertion (checked before value assertions)
	if len(exp.Columns) > 0 {
		assertColumns(t, result, exp.Columns)
	}

	// Value assertions
	expRows := expectedRows(exp.Rows)

	switch {
	case exp.Empty:
		assertEmpty(t, result)
	case exp.Count != nil:
		assertCount(t, result, *exp.Count)
	case exp.Hash != nil:
		assertHash(t, result, *exp.Hash)
	case len(expRows) > 0:
		if exp.Precision != nil {
			assertRowsWithPrecision(t, result, expRows, *exp.Precision)
		} else if exp.Ordered {
			assertRowsOrdered(t, result, expRows)
		} else {
			assertRowsSorted(t, result, expRows)
		}
	// If none of the above, the test just checks the query does not error
	}
}

// runErrorTest executes queries expecting the last one to produce an error.
func runErrorTest(t *testing.T, db *grafeo.Database, language string, queries []string, expectedSubstr string) {
	t.Helper()

	// Execute all-but-last normally
	for _, q := range queries[:len(queries)-1] {
		if _, err := executeQuery(db, language, q); err != nil {
			if isUnsupportedLanguageError(err) {
				t.Skipf("Language %q not available: %v", language, err)
			}
			t.Fatalf("Pre-error statement failed unexpectedly: %v\nQuery: %s", err, q)
		}
	}

	// Last query should fail
	_, err := executeQuery(db, language, queries[len(queries)-1])
	if err == nil {
		t.Fatalf("Expected error containing %q but query succeeded", expectedSubstr)
	}

	errMsg := err.Error()
	if !strings.Contains(errMsg, expectedSubstr) {
		// Case-insensitive fallback
		if !strings.Contains(strings.ToLower(errMsg), strings.ToLower(expectedSubstr)) {
			t.Fatalf("Error %q does not contain %q", errMsg, expectedSubstr)
		}
	}
}

// isUnsupportedLanguageError checks if an error indicates the language is not
// compiled into the grafeo-c library.
func isUnsupportedLanguageError(err error) bool {
	msg := strings.ToLower(err.Error())
	return strings.Contains(msg, "unsupported language") ||
		strings.Contains(msg, "not available") ||
		strings.Contains(msg, "not supported") ||
		strings.Contains(msg, "unknown language")
}
