/// Tests that exercise recently fixed or added C API entry points directly.
///
/// These tests exist to catch regressions in the C FFI layer. Each test maps
/// to a specific C function that was added or fixed:
///
/// - `grafeo_execute_language`             (added: unified dispatcher)
/// - `grafeo_execute_*_with_params`        (added: per-language parameterized variants)
/// - `grafeo_transaction_execute_language` (added: language dispatch in transactions)
/// - `grafeo_free_string`                  (added: caller-owned string free)
library;

import 'package:grafeo/grafeo.dart';
import 'package:test/test.dart';

void main() {
  late GrafeoDB db;

  setUp(() {
    db = GrafeoDB.memory();
  });

  tearDown(() {
    db.close();
  });

  // -------------------------------------------------------------------------
  // grafeo_execute_language
  // -------------------------------------------------------------------------

  group('executeLanguage (grafeo_execute_language)', () {
    test('dispatches GQL without params', () {
      db.execute("INSERT (:Person {name: 'Alix', age: 30})");
      final result = db.executeLanguage(
        'gql',
        'MATCH (p:Person) RETURN p.name',
      );
      expect(result.rows, hasLength(1));
      expect(result.rows.first['p.name'], equals('Alix'));
    });

    test('dispatches GQL with params', () {
      db.execute("INSERT (:City {name: 'Amsterdam'})");
      db.execute("INSERT (:City {name: 'Berlin'})");
      final result = db.executeLanguage(
        'gql',
        r'MATCH (c:City) WHERE c.name = $name RETURN c.name',
        params: {'name': 'Amsterdam'},
      );
      expect(result.rows, hasLength(1));
      expect(result.rows.first['c.name'], equals('Amsterdam'));
    });

    test('null params pointer path (no params argument) is safe', () {
      db.execute("INSERT (:Person {name: 'Gus'})");
      // Calls grafeo_execute_language with a null params_json pointer.
      final result = db.executeLanguage(
        'gql',
        'MATCH (p:Person) RETURN p.name',
      );
      expect(result.rows, hasLength(1));
    });

    test('returns empty result on no-match', () {
      final result = db.executeLanguage('gql', 'MATCH (n:Ghost) RETURN n');
      expect(result.rows, isEmpty);
    });

    test('throws on invalid language', () {
      expect(
        () => db.executeLanguage('bogus', 'MATCH (n) RETURN n'),
        throwsA(isA<GrafeoException>()),
      );
    });

    test('throws on invalid query', () {
      expect(
        () => db.executeLanguage('gql', 'NOT VALID GQL AT ALL'),
        throwsA(isA<GrafeoException>()),
      );
    });
  });

  // -------------------------------------------------------------------------
  // grafeo_execute_cypher_with_params  (and peer _with_params variants)
  // -------------------------------------------------------------------------

  group('executeCypherWithParams (grafeo_execute_cypher_with_params)', () {
    test('executes a Cypher query with parameters', () {
      db.execute("INSERT (:Person {name: 'Vincent', age: 35})");
      final result = db.executeCypherWithParams(
        r'MATCH (p:Person) WHERE p.name = $name RETURN p.name, p.age',
        {'name': 'Vincent'},
      );
      expect(result.rows, hasLength(1));
      expect(result.rows.first['p.name'], equals('Vincent'));
      expect(result.rows.first['p.age'], equals(35));
    });

    test('returns empty result when parameter does not match', () {
      db.execute("INSERT (:Person {name: 'Jules'})");
      final result = db.executeCypherWithParams(
        r'MATCH (p:Person) WHERE p.name = $name RETURN p.name',
        {'name': 'nobody'},
      );
      expect(result.rows, isEmpty);
    });
  });

  // -------------------------------------------------------------------------
  // grafeo_transaction_execute_language
  // -------------------------------------------------------------------------

  group('Transaction.executeLanguage (grafeo_transaction_execute_language)',
      () {
    test('dispatches GQL within a transaction without params', () {
      final tx = db.beginTransaction();
      tx.execute("INSERT (:Person {name: 'Mia'})");
      final result = tx.executeLanguage(
        'gql',
        'MATCH (p:Person) RETURN p.name',
      );
      expect(result.rows, hasLength(1));
      expect(result.rows.first['p.name'], equals('Mia'));
      tx.rollback();
    });

    test('dispatches GQL within a transaction with params', () {
      final tx = db.beginTransaction();
      tx.execute("INSERT (:City {name: 'Prague'})");
      tx.execute("INSERT (:City {name: 'Barcelona'})");
      final result = tx.executeLanguage(
        'gql',
        r'MATCH (c:City) WHERE c.name = $name RETURN c.name',
        params: {'name': 'Prague'},
      );
      expect(result.rows, hasLength(1));
      expect(result.rows.first['c.name'], equals('Prague'));
      tx.rollback();
    });

    test('null params pointer path is safe in transaction', () {
      final tx = db.beginTransaction();
      tx.execute("INSERT (:Person {name: 'Butch'})");
      final result = tx.executeLanguage(
        'gql',
        'MATCH (p:Person) RETURN p.name',
      );
      expect(result.rows.first['p.name'], equals('Butch'));
      tx.rollback();
    });

    test('language changes committed by transaction are visible after commit',
        () {
      final tx = db.beginTransaction();
      tx.executeLanguage(
        'gql',
        "INSERT (:Person {name: 'Django'})",
      );
      tx.commit();

      final result = db.execute('MATCH (p:Person) RETURN p.name');
      expect(result.rows.map((r) => r['p.name']), contains('Django'));
    });

    test('language changes rolled back are not visible', () {
      final tx = db.beginTransaction();
      tx.executeLanguage(
        'gql',
        "INSERT (:Person {name: 'Shosanna'})",
      );
      tx.rollback();

      final result = db.execute('MATCH (p:Person) RETURN p.name');
      expect(result.rows, isEmpty);
    });

    test('throws on finished transaction', () {
      final tx = db.beginTransaction();
      tx.commit();
      expect(
        () => tx.executeLanguage('gql', 'MATCH (n) RETURN n'),
        throwsA(isA<TransactionException>()),
      );
    });
  });

  // -------------------------------------------------------------------------
  // grafeo_free_string  (tested indirectly via db.info())
  // -------------------------------------------------------------------------

  group('grafeo_free_string (via db.info)', () {
    test('info() parses and frees the C-allocated string without leaking', () {
      // Calls grafeo_info (returns heap string) then grafeo_free_string.
      // If free is broken, this will crash under ASAN or produce double-free.
      final info = db.info();
      expect(info, isA<Map<String, dynamic>>());
      expect(info, contains('version'));
      expect(info['version'], isA<String>());
    });

    test('info() can be called multiple times safely', () {
      // Each call allocates and frees a fresh C string.
      for (var i = 0; i < 5; i++) {
        final info = db.info();
        expect(info, contains('version'));
      }
    });
  });
}
