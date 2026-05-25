fn main() {
    // Compile the C file and link it into the Rust cdylib
    cc::Build::new()
        .file("../camel_tokenizer.c")
        .shared_flag(true) // Request a shared library build
        .compile("camel_tokenizer");
    println!("cargo:rustc-link-lib=static=camel_tokenizer");
}
