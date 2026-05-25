// Rust cdylib wrapper for camel_tokenizer.c
// This exposes the required sqlite3_extension_init symbol for SQLite extension loading

#[no_mangle]
#[allow(non_snake_case)]
#[cfg(target_os = "windows")]
pub extern "C" fn sqlite3_extension_init(
    db: *mut std::ffi::c_void,
    pz_err_msg: *mut *mut std::os::raw::c_char,
    p_api: *const std::ffi::c_void,
) -> i32 {
    extern "C" {
        fn sqlite3_camel_init(
            db: *mut std::ffi::c_void,
            pz_err_msg: *mut *mut std::os::raw::c_char,
            p_api: *const std::ffi::c_void,
        ) -> i32;
    }
    unsafe { sqlite3_camel_init(db, pz_err_msg, p_api) }
}
