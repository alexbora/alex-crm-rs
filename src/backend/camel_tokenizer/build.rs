fn main() {
    cc::Build::new()
        .file("../camel_tokenizer.c")
        .compile("camel_tokenizer");
}
