//! Nautilus/Nemo/Caja InfoProvider; main-loop-only weak file tracking.
use super::*;
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicU32, Ordering},
        LazyLock, Mutex,
    },
};
struct InfoApi {
    add: unsafe extern "C" fn(Ptr, *const c_char),
    invalidate: unsafe extern "C" fn(Ptr),
    weak_ref: unsafe extern "C" fn(Ptr, unsafe extern "C" fn(Ptr, Ptr), Ptr),
    weak_unref: unsafe extern "C" fn(Ptr, unsafe extern "C" fn(Ptr, Ptr), Ptr),
    reference: unsafe extern "C" fn(Ptr) -> Ptr,
    unreference: unsafe extern "C" fn(Ptr),
}
static INFO: OnceLock<InfoApi> = OnceLock::new();
static FILES: LazyLock<Mutex<BTreeMap<usize, (String, u8)>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));
static TIMER: AtomicU32 = AtomicU32::new(0);
unsafe extern "C" fn gone(_: Ptr, file: Ptr) {
    FILES.lock().unwrap().remove(&(file as usize));
}
unsafe extern "C" fn tick(_: Ptr) -> c_int {
    let _ = std::panic::catch_unwind(|| {
        let Some(api) = INFO.get() else { return };
        let mut changed = Vec::new();
        {
            let mut files = FILES.lock().unwrap();
            for (file, (path, old)) in files.iter_mut() {
                let now = availability::status(std::path::Path::new(path));
                if now != *old {
                    *old = now;
                    changed.push((api.reference)(*file as Ptr));
                }
            }
        }
        for file in changed {
            (api.invalidate)(file);
            (api.unreference)(file);
        }
    });
    1
}
unsafe extern "C" fn update(_provider: Ptr, file: Ptr, _closure: Ptr, _handle: *mut Ptr) -> c_int {
    std::panic::catch_unwind(|| {
        let (Some(api), Some(info)) = (API.get(), INFO.get()) else {
            return 0;
        };
        let uri = (api.uri)(file);
        if uri.is_null() {
            return 0;
        }
        let mut hostname = ptr::null_mut();
        let local = (api.filename)(uri, &mut hostname, ptr::null_mut());
        (api.free)(uri.cast());
        let local_host = hostname.is_null()
            || CStr::from_ptr(hostname).to_bytes().is_empty()
            || CStr::from_ptr(hostname).to_bytes() == b"localhost";
        if !hostname.is_null() {
            (api.free)(hostname.cast())
        }
        if local.is_null() {
            return 0;
        }
        let path = CStr::from_ptr(local).to_str().ok().map(str::to_owned);
        (api.free)(local.cast());
        let Some(path) = path.filter(|p| {
            local_host
                && p.len() <= 4096
                && paths::configured_root()
                    .is_some_and(|r| paths::relative(&r, std::path::Path::new(p)).is_some())
        }) else {
            return 0;
        };
        let complete = availability::status(std::path::Path::new(&path));
        if complete == 2 {
            (info.add)(file, c"neutrondrive-syncing".as_ptr());
        } else if complete == 1 {
            (info.add)(file, c"neutrondrive-available".as_ptr());
        }
        let mut files = FILES.lock().unwrap();
        if files.len() < 10000 && !files.contains_key(&(file as usize)) {
            (info.weak_ref)(file, gone, ptr::null_mut());
            files.insert(file as usize, (path, complete));
        }
        drop(files);
        if TIMER.load(Ordering::Relaxed) == 0 {
            if let Some(add) = symbol::<
                unsafe extern "C" fn(
                    c_int,
                    c_uint,
                    unsafe extern "C" fn(Ptr) -> c_int,
                    Ptr,
                    Option<unsafe extern "C" fn(Ptr)>,
                ) -> c_uint,
            >("g_timeout_add_seconds_full")
            {
                // Do not extend host provider lifetime into module teardown.
                let id = add(0, 2, tick, ptr::null_mut(), None);
                TIMER.store(id, Ordering::Relaxed);
            }
        }
        0
    })
    .unwrap_or(1)
}
unsafe extern "C" fn cancel(_: Ptr, _: Ptr) {}
unsafe extern "C" fn init(interface: Ptr, _: Ptr) {
    let slots = interface as *mut usize;
    *slots.add(2) = update as *const () as usize;
    *slots.add(3) = cancel as *const () as usize;
}
pub unsafe fn register(
    module: Ptr,
    type_: GType,
    prefix: &str,
    add: unsafe extern "C" fn(Ptr, GType, GType, *const InterfaceInfo),
) {
    let prepare = || -> Option<()> {
        let provider: unsafe extern "C" fn() -> GType =
            symbol(&format!("{prefix}_info_provider_get_type"))?;
        let info = InfoApi {
            add: symbol(&format!("{prefix}_file_info_add_emblem"))?,
            invalidate: symbol(&format!("{prefix}_file_info_invalidate_extension_info"))?,
            weak_ref: symbol("g_object_weak_ref")?,
            weak_unref: symbol("g_object_weak_unref")?,
            reference: symbol("g_object_ref")?,
            unreference: symbol("g_object_unref")?,
        };
        INFO.set(info).ok()?;
        add(
            module,
            type_,
            provider(),
            &InterfaceInfo {
                init: Some(init),
                finalize: ptr::null_mut(),
                data: ptr::null_mut(),
            },
        );
        Some(())
    };
    let _ = prepare();
}
pub fn shutdown() {
    unsafe {
        if let Some(info) = INFO.get() {
            let files = std::mem::take(&mut *FILES.lock().unwrap());
            for (file, _) in files {
                (info.weak_unref)(file as Ptr, gone, ptr::null_mut());
            }
        }
        let id = TIMER.swap(0, Ordering::Relaxed);
        if id != 0 {
            if let Some(remove) = symbol::<unsafe extern "C" fn(c_uint) -> c_int>("g_source_remove")
            {
                remove(id);
            }
        }
    }
}
