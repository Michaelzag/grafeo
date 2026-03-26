import 'dart:convert';
import 'package:grafeo/src/value.dart';
import 'package:test/test.dart';

void main() {
  group('Duration decoding', () {
    test('decodes PT1H30M10S to Duration', () {
      final json = jsonEncode([
        {r'$duration': 'PT1H30M10S'}
      ]);
      final rows = parseRows(json);
      expect(rows, hasLength(1));
      final val = rows[0].values.first;
      expect(val, isA<Duration>());
      expect(val, equals(const Duration(hours: 1, minutes: 30, seconds: 10)));
    });

    test('decodes PT0S to Duration.zero', () {
      final json = jsonEncode([
        {r'$duration': 'PT0S'}
      ]);
      final rows = parseRows(json);
      final val = rows[0].values.first;
      expect(val, isA<Duration>());
      expect(val, equals(Duration.zero));
    });

    test('decodes hours-only duration PT2H', () {
      final json = jsonEncode([
        {r'$duration': 'PT2H'}
      ]);
      final rows = parseRows(json);
      final val = rows[0].values.first;
      expect(val, isA<Duration>());
      expect(val, equals(const Duration(hours: 2)));
    });

    test('decodes minutes-and-seconds PT5M30S', () {
      final json = jsonEncode([
        {r'$duration': 'PT5M30S'}
      ]);
      final rows = parseRows(json);
      final val = rows[0].values.first;
      expect(val, isA<Duration>());
      expect(val, equals(const Duration(minutes: 5, seconds: 30)));
    });

    test('decodes seconds-only PT45S', () {
      final json = jsonEncode([
        {r'$duration': 'PT45S'}
      ]);
      final rows = parseRows(json);
      final val = rows[0].values.first;
      expect(val, isA<Duration>());
      expect(val, equals(const Duration(seconds: 45)));
    });
  });

  group('Duration encoding', () {
    test('encodes Duration to ISO format', () {
      final encoded = encodeParams({'dur': const Duration(hours: 1, minutes: 30)});
      final decoded = jsonDecode(encoded) as Map<String, dynamic>;
      expect(decoded['dur'], isA<Map>());
      expect((decoded['dur'] as Map)[r'$duration'], equals('PT1H30M'));
    });

    test('encodes zero Duration', () {
      final encoded = encodeParams({'dur': Duration.zero});
      final decoded = jsonDecode(encoded) as Map<String, dynamic>;
      expect((decoded['dur'] as Map)[r'$duration'], equals('PT0S'));
    });
  });

  group('Timestamp decoding', () {
    test('decodes timestamp_us marker to DateTime', () {
      final us = DateTime(2024, 1, 15, 12, 0, 0).toUtc().microsecondsSinceEpoch;
      final json = jsonEncode([
        {r'$timestamp_us': us}
      ]);
      final rows = parseRows(json);
      final val = rows[0].values.first;
      expect(val, isA<DateTime>());
    });
  });

  group('Nested maps', () {
    test('decodes nested regular maps', () {
      final json = jsonEncode([
        {
          'outer': {'inner': 'value', 'num': 42}
        }
      ]);
      final rows = parseRows(json);
      final val = rows[0]['outer'] as Map<String, dynamic>;
      expect(val['inner'], equals('value'));
      expect(val['num'], equals(42));
    });
  });
}
