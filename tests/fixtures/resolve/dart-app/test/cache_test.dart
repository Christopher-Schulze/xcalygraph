import 'package:app/store.dart';
import 'package:test/test.dart';

void main() {
  test('reads through', () {
    final cache = cached(MemoryRepository());
    expect(cache.find(1), isNull);
  });
}
