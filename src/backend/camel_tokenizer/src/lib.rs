// Re-export the C tokenizer for DLL build
#[allow(non_snake_case)]
#[no_mangle]
pub extern "C" fn sqlite3_camel_init(
    db: *mut std::ffi::c_void,
    pzErrMsg: *mut *mut std::os::raw::c_char,
    pApi: *const std::ffi::c_void,
) -> i32 {
    extern "C" {
        fn sqlite3_camel_init(
            db: *mut std::ffi::c_void,
            pzErrMsg: *mut *mut std::os::raw::c_char,
            pApi: *const std::ffi::c_void,
        ) -> i32;
    }
    unsafe { sqlite3_camel_init(db, pzErrMsg, pApi) }
}
