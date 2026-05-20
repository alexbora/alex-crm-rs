fn main() {
    cc::Build::new()
        .file("src/backend/camel_tokenizer.c")
        .include("src/backend")
        .compile("camel_tokenizer");
}
