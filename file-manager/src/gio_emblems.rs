//! Optional GVfs metadata bridge for Thunar and the PCManFM family.
//! Runs in a disposable helper process, never on a file-manager UI thread.
use std::{
    ffi::{c_char, c_int, c_void, CStr, CString},
    ptr,
};
type P = *mut c_void;
pub fn apply(path: &str, available: bool) -> Result<(), String> {
    unsafe {
        let library = libc::dlopen(
            c"libgio-2.0.so.0".as_ptr(),
            libc::RTLD_LAZY | libc::RTLD_GLOBAL,
        );
        if library.is_null() {
            return Err("GIO unavailable".into());
        }
        unsafe fn sym<T: Copy>(h: P, n: &CStr) -> Result<T, String> {
            let p = libc::dlsym(h, n.as_ptr());
            if p.is_null() {
                return Err("GIO symbol missing".into());
            }
            Ok(std::mem::transmute_copy(&p))
        }
        let file_new: unsafe extern "C" fn(*const c_char) -> P =
            sym(library, c"g_file_new_for_path")?;
        let query: unsafe extern "C" fn(P, *const c_char, c_int, P, *mut P) -> P =
            sym(library, c"g_file_query_info")?;
        let get: unsafe extern "C" fn(P, *const c_char) -> *mut *mut c_char =
            sym(library, c"g_file_info_get_attribute_stringv")?;
        let set: unsafe extern "C" fn(P, *const c_char, c_int, P, c_int, P, *mut P) -> c_int =
            sym(library, c"g_file_set_attribute")?;
        let unref: unsafe extern "C" fn(P) = sym(library, c"g_object_unref")?;
        let err_free: unsafe extern "C" fn(P) = sym(library, c"g_error_free")?;
        let name = CString::new(path).map_err(|_| "Invalid path")?;
        let file = file_new(name.as_ptr());
        if file.is_null() {
            return Err("Invalid GFile".into());
        }
        let mut error = ptr::null_mut();
        let info = query(
            file,
            c"metadata::emblems".as_ptr(),
            1,
            ptr::null_mut(),
            &mut error,
        );
        if info.is_null() {
            if !error.is_null() {
                err_free(error)
            }
            unref(file);
            return Err("Cannot read file emblems".into());
        }
        let values = get(info, c"metadata::emblems".as_ptr());
        let mut names = Vec::new();
        let mut i = 0;
        if !values.is_null() {
            while i < 256 && !(*values.add(i)).is_null() {
                let s = CStr::from_ptr(*values.add(i));
                if s.to_bytes() != b"neutrondrive-available" {
                    names.push(s.to_owned())
                }
                i += 1;
            }
        }
        if available {
            names.push(CString::new("neutrondrive-available").unwrap())
        }
        let mut argv: Vec<*const c_char> = names.iter().map(|s| s.as_ptr()).collect();
        argv.push(ptr::null());
        let ok = set(
            file,
            c"metadata::emblems".as_ptr(),
            9,
            argv.as_ptr() as P,
            1,
            ptr::null_mut(),
            &mut error,
        ) != 0;
        if !error.is_null() {
            err_free(error)
        }
        unref(info);
        unref(file);
        if ok {
            Ok(())
        } else {
            Err("GVfs emblem metadata is unavailable".into())
        }
    }
}
