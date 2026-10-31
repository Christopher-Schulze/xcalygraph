package com.example.store;

/** Reads rows by id. */
public interface Repository {
    Row find(int id);
}
