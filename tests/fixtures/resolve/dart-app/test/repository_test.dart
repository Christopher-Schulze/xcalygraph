import 'package:app/src/repository.dart';
import 'package:test/test.dart';

void main() {
  test('finds stored rows', () {
    final repository = MemoryRepository();
    repository.put(Row(1));
    expect(repository.find(1), isNotNull);
  });
}
