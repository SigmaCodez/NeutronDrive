//! Hide only our redundant GIO volume; the bookmark remains a normal folder.
use super::*;
use std::sync::atomic::{AtomicU32, Ordering};
struct VolumeApi {
    monitor: unsafe extern "C" fn() -> Ptr,
    mounts: unsafe extern "C" fn(Ptr) -> *mut List,
    root: unsafe extern "C" fn(Ptr) -> Ptr,
    path: unsafe extern "C" fn(Ptr) -> *mut c_char,
    shadow: unsafe extern "C" fn(Ptr),
    unshadow: unsafe extern "C" fn(Ptr),
    unref: unsafe extern "C" fn(Ptr),
    free_list: unsafe extern "C" fn(*mut List),
}
static VOLUME: OnceLock<VolumeApi> = OnceLock::new();
static SHADOWED: std::sync::Mutex<Vec<usize>> = std::sync::Mutex::new(Vec::new());
static TIMER: AtomicU32 = AtomicU32::new(0);
unsafe extern "C" fn poll(_: Ptr) -> c_int {
    let _ = std::panic::catch_unwind(|| {
        let Some(api) = VOLUME.get() else { return };
        let root = paths::configured_root();
        let monitor = (api.monitor)();
        if monitor.is_null() {
            return;
        }
        let mounts = (api.mounts)(monitor);
        let mut item = mounts;
        let mut shadowed = SHADOWED.lock().unwrap();
        let mut current = Vec::new();
        while !item.is_null() {
            let mount = (*item).data;
            let file = (api.root)(mount);
            let path = (api.path)(file);
            let same = !path.is_null()
                && CStr::from_ptr(path)
                    .to_str()
                    .ok()
                    .is_some_and(|p| Some(std::path::Path::new(p)) == root.as_deref());
            if !path.is_null() {
                (API.get().unwrap().free)(path.cast())
            }
            (api.unref)(file);
            if same {
                current.push(mount as usize);
            }
            if same && !shadowed.contains(&(mount as usize)) {
                (api.shadow)(mount);
                shadowed.push(mount as usize);
            } else {
                (api.unref)(mount);
            }
            item = (*item).next;
        }
        shadowed.retain(|mount| {
            if current.contains(mount) {
                true
            } else {
                (api.unshadow)(*mount as Ptr);
                (api.unref)(*mount as Ptr);
                false
            }
        });
        (api.free_list)(mounts);
        (api.unref)(monitor);
        sidebar::refresh();
        sidebar_gtk3::refresh();
    });
    1
}
pub unsafe fn start(provider: Ptr) {
    if TIMER.load(Ordering::Relaxed) != 0 {
        return;
    }
    let init = || -> Option<()> {
        let api = VolumeApi {
            monitor: symbol("g_volume_monitor_get")?,
            mounts: symbol("g_volume_monitor_get_mounts")?,
            root: symbol("g_mount_get_root")?,
            path: symbol("g_file_get_path")?,
            shadow: symbol("g_mount_shadow")?,
            unshadow: symbol("g_mount_unshadow")?,
            unref: symbol("g_object_unref")?,
            free_list: symbol("g_list_free")?,
        };
        let reference: unsafe extern "C" fn(Ptr) -> Ptr = symbol("g_object_ref")?;
        let add: unsafe extern "C" fn(
            c_int,
            c_uint,
            unsafe extern "C" fn(Ptr) -> c_int,
            Ptr,
            unsafe extern "C" fn(Ptr),
        ) -> c_uint = symbol("g_timeout_add_seconds_full")?;
        VOLUME.set(api).ok()?;
        poll(ptr::null_mut());
        let id = add(0, 2, poll, reference(provider), VOLUME.get()?.unref);
        TIMER.store(id, Ordering::Relaxed);
        Some(())
    };
    let _ = init();
}
pub fn shutdown() {
    unsafe {
        sidebar::shutdown();
        sidebar_gtk3::shutdown();
        let id = TIMER.swap(0, Ordering::Relaxed);
        if id != 0 {
            if let Some(remove) = symbol::<unsafe extern "C" fn(c_uint) -> c_int>("g_source_remove")
            {
                remove(id);
            }
        }
        if let Some(api) = VOLUME.get() {
            for mount in std::mem::take(&mut *SHADOWED.lock().unwrap()) {
                (api.unshadow)(mount as Ptr);
                (api.unref)(mount as Ptr);
            }
        }
    }
}
