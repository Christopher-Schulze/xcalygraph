/// Reads rows by id.
abstract class Repository {
  Row? find(int id);
}

class Row {
  final int id;
  Row(this.id);
}

class MemoryRepository implements Repository {
  final Map<int, Row> _rows = {};

  @override
  Row? find(int id) {
    _check(id);
    return _rows[id];
  }

  void put(Row row) {
    _rows[row.id] = row;
  }

  void _check(int id) {}
}
