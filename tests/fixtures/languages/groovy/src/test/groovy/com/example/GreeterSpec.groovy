package com.example

import spock.lang.Specification

class GreeterSpec extends Specification {
    def "greets by name"() {
        given:
        def greeter = new Greeter(new PlainFormatter())

        expect:
        greeter.greet("Ada") == "Hello, Ada"
    }
}
