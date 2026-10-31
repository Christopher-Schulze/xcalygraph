package com.example;

import java.util.List;
import static java.util.Objects.requireNonNull;

/**
 * HTTP endpoints for users.
 */
@RestController
public class Controller {
    public static final String PREFIX = "/users";
    private static final int LIMIT = 10;
    private final UserService users;

    Controller(UserService users) {
        this.users = requireNonNull(users);
    }

    /** Lists all users. */
    @GetMapping(PREFIX)
    public List<User> list() {
        return users.all(LIMIT);
    }

    @Override
    public String toString() {
        return System.getenv("APP_NAME");
    }

    private void unusedHelper() {}

    public enum Mode { READ, WRITE }

    interface Listener {
        void changed(User user);
    }

    public static void main(String[] args) {
        new Controller(new UserService()).list();
    }
}
