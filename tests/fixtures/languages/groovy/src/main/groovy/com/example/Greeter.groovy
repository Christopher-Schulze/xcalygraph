package com.example

/** Builds greetings. */
class Greeter {
    static final String PREFIX = "Hello"
    private final Formatter formatter

    Greeter(Formatter formatter) {
        this.formatter = formatter
    }

    String greet(String name) {
        formatter.format(PREFIX, name)
    }
}

interface Formatter {
    String format(String prefix, String name)
}

class PlainFormatter implements Formatter {
    String format(String prefix, String name) {
        "${prefix}, ${name}"
    }
}
