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
    reference: unsafe extern "C" fn(Ptr) -> Ptr,
    unref: unsafe extern "C" fn(Ptr),
    emit: unsafe extern "C" fn(Ptr, *const c_char, ...),
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
        let mut changed = Vec::new();
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
                changed.push((api.reference)(mount));
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
                changed.push((api.reference)(*mount as Ptr));
                (api.unref)(*mount as Ptr);
                false
            }
        });
        drop(shadowed);
        // Shadow counters do not emit signals. Notify only on transitions and
        // outside our mutex: host callbacks may synchronously inspect providers.
        for mount in changed {
            notify(api, monitor, mount);
            (api.unref)(mount);
        }
        (api.free_list)(mounts);
        (api.unref)(monitor);
        sidebar::refresh();
        sidebar_gtk3::refresh();
    });
    1
}
unsafe fn notify(api: &VolumeApi, monitor: Ptr, mount: Ptr) {
    (api.emit)(mount, c"changed".as_ptr());
    (api.emit)(monitor, c"mount-changed".as_ptr(), mount);
}
pub unsafe fn start(_provider: Ptr) {
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
            reference: symbol("g_object_ref")?,
            unref: symbol("g_object_unref")?,
            emit: symbol("g_signal_emit_by_name")?,
            free_list: symbol("g_list_free")?,
        };
        let add: unsafe extern "C" fn(
            c_int,
            c_uint,
            unsafe extern "C" fn(Ptr) -> c_int,
            Ptr,
            Option<unsafe extern "C" fn(Ptr)>,
        ) -> c_uint = symbol("g_timeout_add_seconds_full")?;
        VOLUME.set(api).ok()?;
        poll(ptr::null_mut());
        // The host owns providers. Keeping one alive until module shutdown can
        // run its weak notifications after the host has freed its provider list.
        // This module-scoped timer has no provider data and is removed on shutdown.
        let id = add(0, 2, poll, ptr::null_mut(), None);
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
            let mounts = std::mem::take(&mut *SHADOWED.lock().unwrap());
            let monitor = (api.monitor)();
            for mount in mounts {
                (api.unshadow)(mount as Ptr);
                if !monitor.is_null() {
                    notify(api, monitor, mount as Ptr);
                }
                (api.unref)(mount as Ptr);
            }
            if !monitor.is_null() {
                (api.unref)(monitor);
            }
        }
    }
}
