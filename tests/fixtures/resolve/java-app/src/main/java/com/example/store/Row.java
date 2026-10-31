package com.example.store;

public record Row(int id, String name) {
    public static final int MAX_ID = 1000;
}
