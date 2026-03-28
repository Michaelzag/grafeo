/// Dart spec runner for .gtest files.
///
/// Discovers all .gtest files under tests/spec/, parses them with the `yaml`
/// package, and creates `package:test` groups and tests that execute queries
/// through the Dart GrafeoDB bindings.
///
/// Run with:
///   dart test spec_runner_test.dart
library;

import 'dart:convert';
import 'dart:io';
import 'dart:math' as math;

import 'package:crypto/crypto.dart' as crypto;
import 'package:grafeo/grafeo.dart';
import 'package:test/test.dart';
import 'package:yaml/yaml.dart';

// =============================================================================
// Paths
// =============================================================================

/// Resolve spec directory relative to this test file.
final _specDir = _resolveSpecDir();
final _datasetsDir = Directory('${_specDir.path}/datasets');

Directory _resolveSpecDir() {
  // Walk up from the working directory until we find Cargo.toml (repo root).
  var dir = Directory.current;
  while (dir.path != dir.parent.path) {
    if (File('${dir.path}/Cargo.toml').existsSync()) {
      return Directory('${dir.path}/tests/spec');
    }
    dir = dir.parent;
  }
  // Fallback: assume cwd is repo root
  return Directory('${Directory.current.path}/tests/spec');
}

// =============================================================================
// Language dispatch
// =============================================================================

/// Normalise a language key to the string accepted by `executeLanguage`.
String _normaliseLanguage(String lang) {
  return switch (lang) {
    'sql_pgq' => 'sql-pgq',
    _ => lang,
  };
}

/// Check whether a language is available by attempting a no-op query.
/// Results are cached per language for the lifetime of the test run.
final _languageAvailability = <String, bool>{};

bool _isLanguageAvailable(String language) {
  final key = _normaliseLanguage(language);
  if (key == 'gql' || key.isEmpty) return true;
  return _languageAvailability.putIfAbsent(key, () {
    final db = GrafeoDB.memory();
    try {
      // Try a minimal query; if the language is not compiled in, this throws.
      db.executeLanguage(key, 'MATCH (n) RETURN n LIMIT 0');
      return true;
    } on GrafeoException {
      return false;
    } finally {
      db.close();
    }
  });
}

// =============================================================================
// Query execution helpers
// =============================================================================

/// Execute a query in the given [language] via `executeLanguage`.
QueryResult _executeQuery(GrafeoDB db, String language, String query) {
  final lang = _normaliseLanguage(language);
  if (lang == 'gql' || lang.isEmpty) {
    return db.execute(query);
  }
  return db.executeLanguage(lang, query);
}

/// Load a .setup dataset file into [db] using GQL.
void _loadDataset(GrafeoDB db, String datasetName) {
  final setupFile = File('${_datasetsDir.path}/$datasetName.setup');
  if (!setupFile.existsSync()) {
    fail('Dataset file not found: ${setupFile.path}');
  }
  final content = setupFile.readAsStringSync();
  for (final line in content.split(RegExp(r'\r?\n'))) {
    final trimmed = line.trim();
    if (trimmed.isEmpty || trimmed.startsWith('#')) continue;
    db.execute(trimmed);
  }
}

// =============================================================================
// Value serialization (canonical format matching Rust runner)
// =============================================================================

/// Convert a Dart value to the canonical string representation used by
/// the Rust spec runner's `value_to_string`.
String _valueToString(dynamic val) {
  if (val == null) return 'null';
  if (val is bool) return val ? 'true' : 'false';
  if (val is int) return val.toString();
  if (val is double) {
    if (val.isNaN) return 'NaN';
    if (val.isInfinite) return val > 0 ? 'Infinity' : '-Infinity';
    // Rust's Display for f64 drops ".0" for whole numbers.
    if (val == val.truncateToDouble() && val.abs() < (1 << 53)) {
      return val.toInt().toString();
    }
    return val.toString();
  }
  if (val is List) {
    final inner = val.map(_valueToString).join(', ');
    return '[$inner]';
  }
  if (val is Map) {
    // Duration: {months, days, nanos} -> ISO 8601
    if (val.length == 3 &&
        val.containsKey('months') &&
        val.containsKey('days') &&
        val.containsKey('nanos')) {
      return _durationToIso(
        (val['months'] as num).toInt(),
        (val['days'] as num).toInt(),
        (val['nanos'] as num).toInt(),
      );
    }
    final entries = val.entries
        .map((e) => '${e.key}: ${_valueToString(e.value)}')
        .toList()
      ..sort();
    return '{${entries.join(', ')}}';
  }
  if (val is DateTime) return val.toIso8601String();
  if (val is Duration) {
    // Format as ISO duration
    final hours = val.inHours;
    final minutes = val.inMinutes.remainder(60);
    final seconds = val.inSeconds.remainder(60);
    final buf = StringBuffer('PT');
    if (hours > 0) buf.write('${hours}H');
    if (minutes > 0) buf.write('${minutes}M');
    if (seconds > 0 || (hours == 0 && minutes == 0)) buf.write('${seconds}S');
    return buf.toString();
  }
  return val.toString();
}

/// Convert {months, days, nanos} to ISO 8601 duration string.
String _durationToIso(int totalMonths, int days, int nanos) {
  final years = totalMonths ~/ 12;
  final months = totalMonths % 12;
  final hours = nanos ~/ 3600000000000;
  var rem = nanos % 3600000000000;
  final minutes = rem ~/ 60000000000;
  rem = rem % 60000000000;
  final seconds = rem ~/ 1000000000;
  final subNanos = rem % 1000000000;

  final buf = StringBuffer('P');
  if (years != 0) buf.write('${years}Y');
  if (months != 0) buf.write('${months}M');
  if (days != 0) buf.write('${days}D');

  final timeBuf = StringBuffer();
  if (hours != 0) timeBuf.write('${hours}H');
  if (minutes != 0) timeBuf.write('${minutes}M');
  if (seconds != 0 || subNanos != 0) {
    if (subNanos != 0) {
      final frac = subNanos.toString().padLeft(9, '0').replaceAll(RegExp(r'0+$'), '');
      timeBuf.write('$seconds.${frac}S');
    } else {
      timeBuf.write('${seconds}S');
    }
  }
  if (timeBuf.isNotEmpty) {
    buf.write('T');
    buf.write(timeBuf);
  }

  final result = buf.toString();
  return result == 'P' ? 'P0D' : result;
}

/// Convert a QueryResult to rows of canonical strings.
List<List<String>> _resultToRows(QueryResult result) {
  final columns = result.columns;
  final rows = <List<String>>[];
  for (final row in result.rows) {
    final r = <String>[];
    for (final col in columns) {
      r.add(_valueToString(row[col]));
    }
    rows.add(r);
  }
  return rows;
}

// =============================================================================
// Assertions
// =============================================================================

/// Assert rows match after sorting both sides.
void _assertRowsSorted(QueryResult result, List<List<String>> expected) {
  final actual = _resultToRows(result);
  final sortedActual = List<List<String>>.from(actual)
    ..sort((a, b) => a.join('|').compareTo(b.join('|')));
  final sortedExpected = List<List<String>>.from(expected)
    ..sort((a, b) => a.join('|').compareTo(b.join('|')));

  expect(sortedActual.length, equals(sortedExpected.length),
      reason:
          'Row count mismatch: got ${sortedActual.length}, '
          'expected ${sortedExpected.length}\n'
          'Actual: $sortedActual\nExpected: $sortedExpected');

  for (var i = 0; i < sortedActual.length; i++) {
    for (var j = 0; j < sortedActual[i].length; j++) {
      expect(sortedActual[i][j], equals(sortedExpected[i][j]),
          reason:
              'Mismatch at sorted row $i, col $j: '
              "got '${sortedActual[i][j]}', expected '${sortedExpected[i][j]}'\n"
              'Actual row: ${sortedActual[i]}\n'
              'Expected row: ${sortedExpected[i]}');
    }
  }
}

/// Assert rows match in exact order.
void _assertRowsOrdered(QueryResult result, List<List<String>> expected) {
  final actual = _resultToRows(result);

  expect(actual.length, equals(expected.length),
      reason:
          'Row count mismatch: got ${actual.length}, '
          'expected ${expected.length}\n'
          'Actual: $actual\nExpected: $expected');

  for (var i = 0; i < actual.length; i++) {
    for (var j = 0; j < actual[i].length; j++) {
      expect(actual[i][j], equals(expected[i][j]),
          reason:
              'Mismatch at row $i, col $j: '
              "got '${actual[i][j]}', expected '${expected[i][j]}'\n"
              'Actual row: ${actual[i]}\n'
              'Expected row: ${expected[i]}');
    }
  }
}

/// Assert rows match with floating-point tolerance.
void _assertRowsWithPrecision(
  QueryResult result,
  List<List<String>> expected,
  int precision,
) {
  final actual = _resultToRows(result);
  final tolerance = math.pow(10, -precision).toDouble();

  expect(actual.length, equals(expected.length),
      reason:
          'Row count mismatch: got ${actual.length}, '
          'expected ${expected.length}');

  for (var i = 0; i < actual.length; i++) {
    for (var j = 0; j < actual[i].length; j++) {
      final a = actual[i][j];
      final e = expected[i][j];
      final af = double.tryParse(a);
      final ef = double.tryParse(e);
      if (af != null && ef != null) {
        expect((af - ef).abs() < tolerance, isTrue,
            reason:
                'Float mismatch at row $i, col $j: '
                'got $af, expected $ef (tolerance $tolerance)');
      } else {
        expect(a, equals(e),
            reason: "Mismatch at row $i, col $j: got '$a', expected '$e'");
      }
    }
  }
}

/// Assert that columns match.
void _assertColumns(QueryResult result, List<String> expected) {
  expect(result.columns, equals(expected),
      reason:
          'Column mismatch: got ${result.columns}, expected $expected');
}

/// Assert that the MD5 hash of sorted, pipe-delimited rows matches.
/// Mirrors assert_hash in the Rust runner.
void _assertHash(QueryResult result, String expectedHash) {
  final rows = _resultToRows(result);
  rows.sort((a, b) => a.join('|').compareTo(b.join('|')));

  final sink = AccumulatorSink<crypto.Digest>();
  final output = crypto.md5.startChunkedConversion(sink);
  for (final row in rows) {
    output.add(utf8.encode('${row.join('|')}\n'));
  }
  output.close();
  final actualHash = sink.events.single.toString();

  expect(actualHash, equals(expectedHash),
      reason: 'Hash mismatch: got $actualHash, expected $expectedHash\n'
          'Rows: $rows');
}

/// Accumulator sink for chunked hash computation.
class AccumulatorSink<T> implements Sink<T> {
  final events = <T>[];

  @override
  void add(T event) => events.add(event);

  @override
  void close() {}
}

// =============================================================================
// .gtest YAML parsing
// =============================================================================

/// Parsed metadata from a .gtest file.
class _Meta {
  String language;
  String dataset;
  List<String> requires;
  List<String> tags;

  _Meta({
    this.language = 'gql',
    this.dataset = 'empty',
    List<String>? requires,
    List<String>? tags,
  })  : requires = requires ?? [],
        tags = tags ?? [];
}

/// Parsed expect block from a test case.
class _Expect {
  List<List<String>> rows;
  bool ordered;
  int? count;
  bool empty;
  String? error;
  String? hash;
  int? precision;
  List<String> columns;

  _Expect({
    List<List<String>>? rows,
    this.ordered = false,
    this.count,
    this.empty = false,
    this.error,
    this.hash,
    this.precision,
    List<String>? columns,
  })  : rows = rows ?? [],
        columns = columns ?? [];
}

/// Parsed test case.
class _TestCase {
  String name;
  String? query;
  List<String> statements;
  List<String> setup;
  String? skip;
  _Expect expect;
  Map<String, String> variants;
  List<String> tags;

  _TestCase({
    this.name = '',
    this.query,
    List<String>? statements,
    List<String>? setup,
    this.skip,
    _Expect? expect,
    Map<String, String>? variants,
    List<String>? tags,
  })  : statements = statements ?? [],
        setup = setup ?? [],
        expect = expect ?? _Expect(),
        variants = variants ?? {},
        tags = tags ?? [];
}

/// Parsed .gtest file.
class _GtestFile {
  final _Meta meta;
  final List<_TestCase> tests;

  _GtestFile(this.meta, this.tests);
}

/// Parse a .gtest YAML file.
_GtestFile _parseGtestFile(File file) {
  final content = file.readAsStringSync();
  final doc = loadYaml(content);
  if (doc is! YamlMap) {
    throw FormatException('Expected YAML mapping at top level in ${file.path}');
  }
  final meta = _parseMeta(doc['meta']);
  final rawTests = doc['tests'];
  final tests = <_TestCase>[];
  if (rawTests is YamlList) {
    for (final raw in rawTests) {
      tests.add(_parseTestCase(raw));
    }
  }
  return _GtestFile(meta, tests);
}

_Meta _parseMeta(dynamic d) {
  if (d == null || d is! YamlMap) return _Meta();
  return _Meta(
    language: _asString(d['language'], 'gql'),
    dataset: _asString(d['dataset'], 'empty'),
    requires: _asStringList(d['requires']),
    tags: _asStringList(d['tags']),
  );
}

_TestCase _parseTestCase(dynamic d) {
  if (d is! YamlMap) return _TestCase();
  final tc = _TestCase(
    name: _asString(d['name'], ''),
    skip: d['skip']?.toString(),
    tags: _asStringList(d['tags']),
  );

  // query
  final q = d['query'];
  if (q != null) tc.query = q.toString().trim();

  // setup / statements
  tc.setup = _asStringList(d['setup']);
  tc.statements = _asStringList(d['statements']);

  // variants
  final rawVariants = d['variants'];
  if (rawVariants is YamlMap) {
    tc.variants = {
      for (final entry in rawVariants.entries)
        entry.key.toString(): entry.value.toString().trim(),
    };
  }

  // expect
  final rawExpect = d['expect'];
  if (rawExpect is YamlMap) {
    tc.expect = _parseExpect(rawExpect);
  }

  return tc;
}

_Expect _parseExpect(YamlMap d) {
  final e = _Expect(
    ordered: d['ordered'] == true,
    empty: d['empty'] == true,
    columns: _asStringList(d['columns']),
  );

  final count = d['count'];
  if (count != null) e.count = count is int ? count : int.parse(count.toString());

  final error = d['error'];
  if (error != null) e.error = error.toString();

  final hashVal = d['hash'];
  if (hashVal != null) e.hash = hashVal.toString();

  final precision = d['precision'];
  if (precision != null) {
    e.precision = precision is int ? precision : int.parse(precision.toString());
  }

  // rows: list of lists
  final rawRows = d['rows'];
  if (rawRows is YamlList) {
    for (final rawRow in rawRows) {
      if (rawRow is YamlList) {
        e.rows.add([for (final v in rawRow) _yamlValueToString(v)]);
      } else {
        // Single-column shorthand
        e.rows.add([_yamlValueToString(rawRow)]);
      }
    }
  }

  return e;
}

/// Convert a YAML-parsed value to the canonical string representation.
/// This handles the type coercions that YAML introduces (e.g. bool, int,
/// float, null) so expected values match the Rust runner's output.
String _yamlValueToString(dynamic val) {
  if (val == null) return 'null';
  if (val is bool) return val ? 'true' : 'false';
  if (val is int) return val.toString();
  if (val is double) {
    if (val.isNaN) return 'NaN';
    if (val.isInfinite) return val > 0 ? 'Infinity' : '-Infinity';
    // Drop trailing .0 for whole numbers (matches Rust Display for f64).
    if (val == val.truncateToDouble() && val.abs() < (1 << 53)) {
      return val.toInt().toString();
    }
    return val.toString();
  }
  if (val is List) {
    final inner = val.map(_yamlValueToString).join(', ');
    return '[$inner]';
  }
  if (val is Map) {
    final entries = val.entries
        .map((e) => '${e.key}: ${_yamlValueToString(e.value)}')
        .toList()
      ..sort();
    return '{${entries.join(', ')}}';
  }
  return val.toString();
}

// =============================================================================
// YAML utility helpers
// =============================================================================

String _asString(dynamic val, String defaultValue) {
  if (val == null) return defaultValue;
  return val.toString();
}

List<String> _asStringList(dynamic val) {
  if (val == null) return [];
  if (val is String) return [val];
  if (val is YamlList) {
    return [for (final v in val) v?.toString().trim() ?? ''];
  }
  if (val is List) {
    return [for (final v in val) v?.toString().trim() ?? ''];
  }
  return [val.toString()];
}

// =============================================================================
// File discovery
// =============================================================================

/// Recursively find all .gtest files under [dir].
List<File> _findGtestFiles(Directory dir) {
  final results = <File>[];
  if (!dir.existsSync()) return results;
  for (final entity in dir.listSync(recursive: true, followLinks: false)) {
    if (entity is File && entity.path.endsWith('.gtest')) {
      // Skip anything inside the runners directory.
      final normalized = entity.path.replaceAll('\\', '/');
      if (normalized.contains('/runners/')) continue;
      results.add(entity);
    }
  }
  results.sort((a, b) => a.path.compareTo(b.path));
  return results;
}

// =============================================================================
// Test runner core
// =============================================================================

/// Run a single test case against [db].
void _runTestCase(GrafeoDB db, _TestCase tc, String language) {
  // Run setup queries in the file's declared language
  for (final setupQ in tc.setup) {
    _executeQuery(db, language, setupQ);
  }

  final exp = tc.expect;

  // Determine queries
  final queries = tc.statements.isNotEmpty
      ? tc.statements
      : tc.query != null
          ? [tc.query!]
          : <String>[];
  if (queries.isEmpty) {
    fail("No query or statements in test '${tc.name}'");
  }

  // Error case
  if (exp.error != null) {
    _runErrorTest(db, language, queries, exp.error!);
    return;
  }

  // Execute all queries, capture last result
  late QueryResult result;
  for (final q in queries) {
    result = _executeQuery(db, language, q);
  }

  // Column assertion (checked before value assertions)
  if (exp.columns.isNotEmpty) {
    _assertColumns(result, exp.columns);
  }

  // Empty check
  if (exp.empty) {
    expect(result.rows.length, equals(0),
        reason: 'Expected empty result, got ${result.rows.length} row(s)');
    return;
  }

  // Count check
  if (exp.count != null) {
    expect(result.rows.length, equals(exp.count),
        reason:
            'Row count mismatch: got ${result.rows.length}, '
            'expected ${exp.count}');
    return;
  }

  // Hash check
  if (exp.hash != null) {
    _assertHash(result, exp.hash!);
    return;
  }

  // Rows check
  if (exp.rows.isNotEmpty) {
    if (exp.precision != null) {
      _assertRowsWithPrecision(result, exp.rows, exp.precision!);
    } else if (exp.ordered) {
      _assertRowsOrdered(result, exp.rows);
    } else {
      _assertRowsSorted(result, exp.rows);
    }
  }
}

/// Run the error test path: execute queries expecting the last to throw.
void _runErrorTest(
  GrafeoDB db,
  String language,
  List<String> queries,
  String expectedSubstring,
) {
  // Execute all-but-last normally
  for (var i = 0; i < queries.length - 1; i++) {
    _executeQuery(db, language, queries[i]);
  }

  // Last query should fail
  try {
    _executeQuery(db, language, queries.last);
    fail("Expected error containing '$expectedSubstring' but query succeeded");
  } on GrafeoException catch (e) {
    expect(e.message, contains(expectedSubstring),
        reason:
            "Error '${e.message}' does not contain '$expectedSubstring'");
  }
}

// =============================================================================
// Main: discover files and register tests
// =============================================================================

void main() {
  final gtestFiles = _findGtestFiles(_specDir);

  if (gtestFiles.isEmpty) {
    test('no .gtest files found', () {
      // Not a failure: the spec files may not be present in all environments
      markTestSkipped('No .gtest files found under ${_specDir.path}');
    });
    return;
  }

  for (final file in gtestFiles) {
    final relPath = file.path
        .replaceAll('\\', '/')
        .replaceFirst('${_specDir.path.replaceAll('\\', '/')}/', '');

    late _GtestFile parsed;
    try {
      parsed = _parseGtestFile(file);
    } catch (e) {
      group(relPath, () {
        test('should parse without errors', () {
          fail('Parse error: $e');
        });
      });
      continue;
    }

    final meta = parsed.meta;

    group(relPath, () {
      for (final tc in parsed.tests) {
        // Handle rosetta variants
        if (tc.variants.isNotEmpty) {
          for (final entry in tc.variants.entries) {
            final lang = entry.key;
            final variantQuery = entry.value;
            test('${tc.name}_$lang', () {
              if (!_isLanguageAvailable(lang)) {
                markTestSkipped('Language "$lang" not available');
                return;
              }
              final db = GrafeoDB.memory();
              try {
                if (meta.dataset.isNotEmpty && meta.dataset != 'empty') {
                  _loadDataset(db, meta.dataset);
                }
                // Create a copy of the test case with the variant query
                final variantTc = _TestCase(
                  name: '${tc.name}_$lang',
                  query: variantQuery,
                  setup: tc.setup,
                  expect: tc.expect,
                  tags: tc.tags,
                );
                _runTestCase(db, variantTc, lang);
              } finally {
                db.close();
              }
            });
          }
          continue;
        }

        test(tc.name, () {
          // Skip if test has a skip field
          if (tc.skip != null) {
            markTestSkipped('skipped in .gtest: ${tc.skip}');
            return;
          }

          // Check language availability
          if (!_isLanguageAvailable(meta.language)) {
            markTestSkipped(
              'Language "${meta.language}" not available',
            );
            return;
          }

          // Check requires
          for (final req in meta.requires) {
            if (!_isLanguageAvailable(req)) {
              markTestSkipped(
                'Required language "$req" not available',
              );
              return;
            }
          }

          final db = GrafeoDB.memory();
          try {
            // Load dataset
            if (meta.dataset.isNotEmpty && meta.dataset != 'empty') {
              _loadDataset(db, meta.dataset);
            }

            _runTestCase(db, tc, meta.language);
          } finally {
            db.close();
          }
        });
      }
    });
  }
}
