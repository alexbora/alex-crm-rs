fn main() {
    // Build camel_tokenizer.dll for SQLite FTS5 extension loading.
    // The DLL is loaded at runtime via rusqlite::Connection::load_extension().

    println!("cargo:rerun-if-changed=src/backend/camel_tokenizer.c");
    println!("cargo:rerun-if-changed=src/backend/camel_tokenizer/Makefile");

    let make_result = std::process::Command::new("make")
        .args(["-C", "src/backend/camel_tokenizer"])
        .status();

    match make_result {
        Ok(status) if status.success() => {
            // Copy DLL to the target output directory alongside the executable.
            // The TARGET env var is e.g. "x86_64-pc-windows-gnu" (set in .cargo/config.toml).
            let profile = std::env::var("PROFILE").unwrap_or_else(|_| "debug".to_string());
            let target_triple = std::env::var("TARGET").unwrap_or_default();
            let mut target_path = std::path::PathBuf::from("target");
            if !target_triple.is_empty() {
                target_path.push(&target_triple);
            }
            target_path.push(&profile);

            let src_dll = std::path::Path::new("src/backend/camel_tokenizer/camel_tokenizer.dll");
            let dst_dll = target_path.join("camel_tokenizer.dll");

            if src_dll.exists() {
                std::fs::create_dir_all(&target_path).ok();
                std::fs::copy(src_dll, &dst_dll).ok();
                println!(
                    "cargo:warning=camel_tokenizer.dll copied to {}",
                    dst_dll.display()
                );
            }
        }
        Ok(_) => {
            println!("cargo:warning=camel_tokenizer.dll build FAILED (make returned non-zero)");
        }
        Err(e) => {
            println!("cargo:warning=camel_tokenizer.dll skipped (make not available: {e})");
        }
    }
}
