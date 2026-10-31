package com.example.app;

import com.example.store.MemoryRepository;
import com.example.store.Repository;
import com.example.store.Row;
import static com.example.store.Row.MAX_ID;

public class Service {
    private final Repository repository;

    public Service(Repository repository) {
        this.repository = repository;
    }

    public String name(int id) {
        Row row = repository.find(id);
        return format(row);
    }

    private String format(Row row) {
        return row.name() + MAX_ID;
    }

    public static void main(String[] args) {
        var service = new Service(new MemoryRepository());
        String url = System.getenv("APP_URL");
        System.out.println(service.name(1) + url);
    }
}
