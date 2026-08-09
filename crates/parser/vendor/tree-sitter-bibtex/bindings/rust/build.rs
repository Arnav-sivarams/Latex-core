fn main() {
    let parser = std::path::Path::new("src/parser.c");
    cc::Build::new()
        .std("c11")
        .include("src")
        .file(parser)
        .compile("tree-sitter-bibtex");
    println!("cargo:rerun-if-changed={}", parser.display());
}
