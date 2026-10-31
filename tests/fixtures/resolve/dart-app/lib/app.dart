import 'package:app/src/repository.dart';

String label(Repository repository, int id) {
  return repository.find(id)?.toString() ?? '';
}

void main() {
  final repository = MemoryRepository();
  repository.put(Row(1));
  print(label(repository, 1));
}
