part of '../store.dart';

class Cache {
  final Repository inner;
  Cache(this.inner);

  Row? find(int id) => inner.find(id);
}
