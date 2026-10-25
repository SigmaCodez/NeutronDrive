//! Local GVfs custom-folder icons for Caja and Thunar. Called only by the
//! disposable, timeout-bounded helper, never in a file-manager UI thread.
use std::{
    ffi::{c_char, c_int, c_void, CStr, CString},
    path::{Path, PathBuf},
    ptr,
};
type P = *mut c_void;
fn asset() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let prefix = exe.parent()?.parent()?.parent()?.parent()?;
    let file =
        prefix.join("share/icons/hicolor/scalable/apps/io.github.SigmaCodez.NeutronDrive.svg");
    file.is_file().then_some(file)
}
pub fn apply(root: &Path) -> Result<(), String> {
    let asset = asset().ok_or("Installed icon missing")?;
    unsafe {
        let library = libc::dlopen(
            c"libgio-2.0.so.0".as_ptr(),
            libc::RTLD_LAZY | libc::RTLD_GLOBAL,
        );
        if library.is_null() {
            return Err("GIO unavailable".into());
        }
        unsafe fn sym<T: Copy>(lib: P, name: &CStr) -> Result<T, String> {
            let p = libc::dlsym(lib, name.as_ptr());
            if p.is_null() {
                return Err("Missing GIO symbol".into());
            }
            Ok(std::mem::transmute_copy(&p))
        }
        let file_new: unsafe extern "C" fn(*const c_char) -> P =
            sym(library, c"g_file_new_for_path")?;
        let uri: unsafe extern "C" fn(P) -> *mut c_char = sym(library, c"g_file_get_uri")?;
        let query: unsafe extern "C" fn(P, *const c_char, c_int, P, *mut P) -> P =
            sym(library, c"g_file_query_info")?;
        let get: unsafe extern "C" fn(P, *const c_char) -> *const c_char =
            sym(library, c"g_file_info_get_attribute_string")?;
        let set: unsafe extern "C" fn(P, *const c_char, c_int, P, c_int, P, *mut P) -> c_int =
            sym(library, c"g_file_set_attribute")?;
        let unref: unsafe extern "C" fn(P) = sym(library, c"g_object_unref")?;
        let free: unsafe extern "C" fn(P) = sym(library, c"g_free")?;
        let err_free: unsafe extern "C" fn(P) = sym(library, c"g_error_free")?;
        use std::os::unix::ffi::OsStrExt;
        let path = CString::new(root.as_os_str().as_bytes()).map_err(|_| "Invalid root")?;
        let icon_path = CString::new(asset.as_os_str().as_bytes()).map_err(|_| "Invalid icon")?;
        let icon_file = file_new(icon_path.as_ptr());
        let icon_uri = uri(icon_file);
        unref(icon_file);
        if icon_uri.is_null() {
            return Err("Invalid icon URI".into());
        }
        let file = file_new(path.as_ptr());
        let mut error = ptr::null_mut();
        let info = query(
            file,
            c"metadata::custom-icon,metadata::thunar-custom-icon-name".as_ptr(),
            1,
            ptr::null_mut(),
            &mut error,
        );
        if info.is_null() {
            if !error.is_null() {
                err_free(error)
            }
            free(icon_uri.cast());
            unref(file);
            return Err("Cannot read folder icon".into());
        }
        let mut ok = true;
        for (key, value) in [
            (c"metadata::custom-icon", CStr::from_ptr(icon_uri)),
            (
                c"metadata::thunar-custom-icon-name",
                c"io.github.SigmaCodez.NeutronDrive",
            ),
        ] {
            let old = get(info, key.as_ptr());
            // Respect a separately chosen user icon. Refresh only unset or
            // NeutronDrive-owned values (including a relocated AppImage).
            if !old.is_null() {
                let old = CStr::from_ptr(old).to_bytes();
                if old == value.to_bytes() {
                    continue;
                }
                if !old.is_empty()
                    && !old.ends_with(b"io.github.SigmaCodez.NeutronDrive.svg")
                    && old != b"io.github.SigmaCodez.NeutronDrive"
                {
                    continue;
                }
            }
            error = ptr::null_mut();
            ok &= set(
                file,
                key.as_ptr(),
                1,
                value.as_ptr() as P,
                1,
                ptr::null_mut(),
                &mut error,
            ) != 0;
            if !error.is_null() {
                err_free(error)
            }
        }
        unref(info);
        unref(file);
        free(icon_uri.cast());
        if ok {
            Ok(())
        } else {
            Err("Cannot set folder icon".into())
        }
    }
}
