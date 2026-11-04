fn main() {
    println!("cargo:rerun-if-changed=src/parser.c");
    cc::Build::new()
        .std("c11")
        .include("src")
        .file("src/parser.c")
        .compile("tree-sitter-robot");
}
