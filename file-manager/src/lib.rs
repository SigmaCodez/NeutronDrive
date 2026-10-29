//! Native menu providers for Nautilus 43+, Nemo, Caja and Thunar 4.16+.
//! Only the host's public GObject ABI is used. No Qt, Python, network calls or
//! cloud operations run in a file-manager process. Activation launches the app
//! with literal argv; the app validates the live mount before doing any work.
mod availability;
mod emblems;
pub mod gio_emblems;
pub mod paths;
pub mod root_icon;
mod sidebar;
mod sidebar_gtk3;
mod volumes;
pub mod watcher;
use std::{
    ffi::{c_char, c_int, c_uint, c_void, CStr, CString},
    ptr,
    sync::OnceLock,
};
type Ptr = *mut c_void;
type GType = usize;
#[repr(C)]
struct List {
    data: Ptr,
    next: *mut List,
    prev: *mut List,
}
#[repr(C)]
struct TypeQuery {
    type_: GType,
    name: *const c_char,
    class_size: c_uint,
    instance_size: c_uint,
}
#[repr(C)]
struct TypeInfo {
    class_size: u16,
    base_init: Ptr,
    base_finalize: Ptr,
    class_init: Ptr,
    class_finalize: Ptr,
    class_data: Ptr,
    instance_size: u16,
    n_preallocs: u16,
    instance_init: Ptr,
    value_table: Ptr,
}
#[repr(C)]
struct InterfaceInfo {
    init: Option<unsafe extern "C" fn(Ptr, Ptr)>,
    finalize: Ptr,
    data: Ptr,
}
#[derive(Clone, Copy, PartialEq)]
enum Host {
    Nautilus,
    #[cfg(feature = "nautilus3")]
    NautilusLegacy,
    Nemo,
    Caja,
    Thunar,
}
struct Api {
    host: Host,
    type_: GType,
    uri: unsafe extern "C" fn(Ptr) -> *mut c_char,
    item: unsafe extern "C" fn(*const c_char, *const c_char, *const c_char, *const c_char) -> Ptr,
    append: unsafe extern "C" fn(*mut List, Ptr) -> *mut List,
    free: unsafe extern "C" fn(Ptr),
    filename: unsafe extern "C" fn(*const c_char, *mut *mut c_char, Ptr) -> *mut c_char,
    spawn: unsafe extern "C" fn(
        *const c_char,
        *mut *mut c_char,
        *mut *mut c_char,
        c_int,
        Ptr,
        Ptr,
        *mut c_int,
        Ptr,
    ) -> c_int,
    connect: unsafe extern "C" fn(
        Ptr,
        *const c_char,
        Ptr,
        Ptr,
        Option<unsafe extern "C" fn(Ptr, Ptr)>,
        c_uint,
    ) -> u64,
}
static API: OnceLock<Api> = OnceLock::new();
static LABELS: OnceLock<[String; 3]> = OnceLock::new();
// dlsym resolves only symbols already loaded by the owning file manager.
unsafe fn symbol<T: Copy>(name: &str) -> Option<T> {
    let name = CString::new(name).ok()?;
    let p = libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr());
    if p.is_null() {
        None
    } else {
        Some(std::mem::transmute_copy(&p))
    }
}
unsafe extern "C" fn interface_init(interface: Ptr, _data: Ptr) {
    // GTypeInterface consists of two GTypes, followed by get_file_items.
    let slot = (interface as *mut usize).add(2);
    *slot = if API.get().is_some_and(|a| a.host == Host::Nautilus) {
        items as *const () as usize
    } else {
        items_with_window as *const () as usize
    };
}
unsafe extern "C" fn instance_ready(instance: Ptr, _: Ptr) {
    volumes::start(instance);
}
unsafe fn initialize(module: Ptr, host: Host) {
    if API.get().is_some() {
        return;
    }
    let prefix = match host {
        Host::Nautilus => "nautilus",
        #[cfg(feature = "nautilus3")]
        Host::NautilusLegacy => "nautilus",
        Host::Nemo => "nemo",
        Host::Caja => "caja",
        Host::Thunar => "thunarx",
    };
    let init = || -> Option<()> {
        let query: unsafe extern "C" fn(GType, *mut TypeQuery) = symbol("g_type_query")?;
        let object: unsafe extern "C" fn() -> GType = symbol("g_object_get_type")?;
        let provider: unsafe extern "C" fn() -> GType =
            symbol(&format!("{prefix}_menu_provider_get_type"))?;
        let register: unsafe extern "C" fn(
            Ptr,
            GType,
            *const c_char,
            *const TypeInfo,
            c_uint,
        ) -> GType = symbol(if host == Host::Thunar {
            "thunarx_provider_plugin_register_type"
        } else {
            "g_type_module_register_type"
        })?;
        let add: unsafe extern "C" fn(Ptr, GType, GType, *const InterfaceInfo) =
            symbol(if host == Host::Thunar {
                "thunarx_provider_plugin_add_interface"
            } else {
                "g_type_module_add_interface"
            })?;
        let mut q = TypeQuery {
            type_: 0,
            name: ptr::null(),
            class_size: 0,
            instance_size: 0,
        };
        query(object(), &mut q);
        let info = TypeInfo {
            class_size: q.class_size.try_into().ok()?,
            base_init: ptr::null_mut(),
            base_finalize: ptr::null_mut(),
            class_init: ptr::null_mut(),
            class_finalize: ptr::null_mut(),
            class_data: ptr::null_mut(),
            instance_size: q.instance_size.try_into().ok()?,
            n_preallocs: 0,
            instance_init: instance_ready as *const () as Ptr,
            value_table: ptr::null_mut(),
        };
        let name = CString::new(format!("NeutronDrive{prefix}MenuProvider")).ok()?;
        // Resolve every callable function before registering the type.
        let mut api = Api {
            host,
            type_: 0,
            uri: symbol(&format!("{prefix}_file_info_get_uri"))?,
            item: symbol(&format!("{prefix}_menu_item_new"))?,
            append: symbol("g_list_append")?,
            free: symbol("g_free")?,
            filename: symbol("g_filename_from_uri")?,
            connect: symbol("g_signal_connect_data")?,
            spawn: symbol("g_spawn_async")?,
        };
        api.type_ = register(module, object(), name.as_ptr(), &info, 0);
        if api.type_ == 0 {
            return None;
        }
        let type_ = api.type_;
        API.set(api).ok()?;
        let iface = InterfaceInfo {
            init: Some(interface_init),
            finalize: ptr::null_mut(),
            data: ptr::null_mut(),
        };
        add(module, type_, provider(), &iface);
        if host != Host::Thunar {
            emblems::register(module, type_, prefix, add);
        }
        Some(())
    };
    let _ = std::panic::catch_unwind(init);
}
struct Action {
    mode: &'static str,
    paths: Vec<String>,
}
unsafe extern "C" fn destroy(data: Ptr, _closure: Ptr) {
    drop(Box::from_raw(data as *mut Action));
}
unsafe extern "C" fn activate(_item: Ptr, data: Ptr) {
    let _ = std::panic::catch_unwind(|| {
        let action = &*(data as *const Action);
        let Some(api) = API.get() else {
            return;
        };
        let values = std::iter::once("/usr/bin/neutrondrive")
            .chain(["--file-action", action.mode, "--"])
            .chain(action.paths.iter().map(String::as_str));
        let Ok(arguments) = values.map(CString::new).collect::<Result<Vec<_>, _>>() else {
            return;
        };
        let mut argv: Vec<_> = arguments
            .iter()
            .map(|s| s.as_ptr() as *mut c_char)
            .collect();
        argv.push(ptr::null_mut());
        // GLib's default detached-child mode handles reaping. No plugin-owned
        // thread or callback may outlive the file manager's module reference.
        (api.spawn)(
            ptr::null(),
            argv.as_mut_ptr(),
            ptr::null_mut(),
            0,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
        );
    });
}
unsafe extern "C" fn items_with_window(provider: Ptr, _window: Ptr, files: *mut List) -> *mut List {
    items(provider, files)
}
unsafe extern "C" fn items(provider: Ptr, files: *mut List) -> *mut List {
    volumes::start(provider);
    std::panic::catch_unwind(|| menu(files)).unwrap_or(ptr::null_mut())
}
unsafe fn menu(mut files: *mut List) -> *mut List {
    let Some(api) = API.get() else {
        return ptr::null_mut();
    };
    let Some(root) = paths::configured_root() else {
        return ptr::null_mut();
    };
    let mut selected = Vec::new();
    while !files.is_null() {
        if selected.len() >= 128 {
            return ptr::null_mut();
        }
        let uri = (api.uri)((*files).data);
        if uri.is_null() {
            return ptr::null_mut();
        }
        let mut hostname: *mut c_char = ptr::null_mut();
        let local = (api.filename)(uri, &mut hostname, ptr::null_mut());
        (api.free)(uri.cast());
        let local_host = hostname.is_null()
            || CStr::from_ptr(hostname).to_bytes().is_empty()
            || CStr::from_ptr(hostname).to_bytes() == b"localhost";
        if !hostname.is_null() {
            (api.free)(hostname.cast());
        }
        if local.is_null() {
            return ptr::null_mut();
        }
        let path = CStr::from_ptr(local).to_str().ok().map(str::to_owned);
        (api.free)(local.cast());
        let Some(path) = path.filter(|p| {
            local_host
                && p.len() <= 4096
                && paths::relative(&root, std::path::Path::new(p)).is_some()
        }) else {
            return ptr::null_mut();
        };
        selected.push(path);
        files = (*files).next;
    }
    if selected.is_empty() {
        return ptr::null_mut();
    }
    let language = std::env::var("LC_ALL")
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(|| std::env::var("LC_MESSAGES").ok().filter(|v| !v.is_empty()))
        .or_else(|| std::env::var("LANG").ok())
        .unwrap_or_default();
    let labels = LABELS.get_or_init(|| {
        let german = language.starts_with("de");
        let mut labels = if german {
            [
                "Auf Gerät behalten".to_string(),
                "Speicher freigeben".into(),
                "Teilen…".into(),
            ]
        } else {
            [
                "Keep on device".to_string(),
                "Free up space".into(),
                "Share…".into(),
            ]
        };
        let locale = language
            .split('.')
            .next()
            .unwrap_or("en")
            .split('@')
            .next()
            .unwrap_or("en")
            .replace('_', "-");
        if locale.chars().all(|c| c.is_ascii_alphabetic() || c == '-') {
            for code in [locale.as_str(), locale.split('-').next().unwrap_or("en")] {
                let path = std::path::Path::new("/usr/lib/neutrondrive/neutrondrive/locales")
                    .join(format!("{code}.json"));
                if let Ok(file) = std::fs::File::open(path) {
                    use std::io::Read;
                    let mut source = String::new();
                    if file.take(262145).read_to_string(&mut source).is_ok()
                        && source.len() <= 262144
                    {
                        if let Ok(catalog) = serde_json::from_str::<serde_json::Value>(&source) {
                            for (index, key) in ["Keep on device", "Free up space", "Freigabe"]
                                .iter()
                                .enumerate()
                            {
                                if let Some(text) = catalog[key]
                                    .as_str()
                                    .filter(|s| !s.contains('\0') && !s.is_empty())
                                {
                                    if index != 2 || !german {
                                        labels[index] = text.into();
                                    }
                                }
                            }
                            break;
                        }
                    }
                }
            }
        }
        labels
    });
    let mut list = ptr::null_mut();
    for (index, (mode, icon)) in [
        ("pin", "folder-download-symbolic"),
        ("evict", "edit-clear-symbolic"),
        ("share", "document-send-symbolic"),
    ]
    .into_iter()
    .enumerate()
    {
        if mode == "share" && selected.len() != 1 {
            continue;
        }
        let id = CString::new(format!("NeutronDrive::{mode}")).unwrap();
        let label = CString::new(labels[index].as_str()).unwrap();
        let icon = CString::new(icon).unwrap();
        let item = (api.item)(
            id.as_ptr(),
            label.as_ptr(),
            c"NeutronDrive".as_ptr(),
            icon.as_ptr(),
        );
        if item.is_null() {
            continue;
        }
        let action = Box::into_raw(Box::new(Action {
            mode,
            paths: selected.clone(),
        }));
        (api.connect)(
            item,
            c"activate".as_ptr(),
            activate as *const () as Ptr,
            action.cast(),
            Some(destroy),
            0,
        );
        list = (api.append)(list, item);
    }
    list
}
unsafe fn list_types(types: *mut *const GType, count: *mut c_int) {
    if types.is_null() || count.is_null() {
        return;
    }
    if let Some(api) = API.get() {
        *types = &api.type_;
        *count = 1
    } else {
        *types = ptr::null();
        *count = 0
    }
}
macro_rules! exports {
    ($init:ident,$shutdown:ident,$list:ident,$host:expr) => {
        /// # Safety
        /// Called by the matching file manager with a live GTypeModule (or Thunar provider plugin).
        #[no_mangle]
        pub unsafe extern "C" fn $init(module: Ptr) {
            initialize(module, $host)
        }
        #[no_mangle]
        pub extern "C" fn $shutdown() {
            emblems::shutdown();
            volumes::shutdown();
        }
        /// # Safety
        /// The host provides writable output pointers and retains the loaded module.
        #[no_mangle]
        pub unsafe extern "C" fn $list(types: *mut *const GType, count: *mut c_int) {
            list_types(types, count)
        }
    };
}
#[cfg(not(feature = "nautilus3"))]
exports!(
    nautilus_module_initialize,
    nautilus_module_shutdown,
    nautilus_module_list_types,
    Host::Nautilus
);
exports!(
    nemo_module_initialize,
    nemo_module_shutdown,
    nemo_module_list_types,
    Host::Nemo
);
exports!(
    caja_module_initialize,
    caja_module_shutdown,
    caja_module_list_types,
    Host::Caja
);
exports!(
    thunar_extension_initialize,
    thunar_extension_shutdown,
    thunar_extension_list_types,
    Host::Thunar
);

#[cfg(feature = "nautilus3")]
exports!(
    nautilus_module_initialize,
    nautilus_module_shutdown,
    nautilus_module_list_types,
    Host::NautilusLegacy
);

/// # Safety
/// The caller supplies a readable NUL-terminated UTF-8 path.
#[no_mangle]
pub unsafe extern "C" fn neutrondrive_is_available(path: *const c_char) -> c_int {
    if path.is_null() {
        return 0;
    }
    std::panic::catch_unwind(|| {
        CStr::from_ptr(path)
            .to_str()
            .ok()
            .filter(|p| p.len() <= 4096)
            .is_some_and(|p| availability::available(std::path::Path::new(p))) as c_int
    })
    .unwrap_or(0)
}

/// # Safety
/// The caller supplies a readable NUL-terminated UTF-8 path.
/// Low two bits: 0 online/unknown, 1 locally available, 2 downloading.
/// Flag 0x04 marks a directly shared folder and can coexist with availability.
#[no_mangle]
pub unsafe extern "C" fn neutrondrive_file_status(path: *const c_char) -> c_int {
    if path.is_null() {
        return 0;
    }
    std::panic::catch_unwind(|| {
        CStr::from_ptr(path)
            .to_str()
            .ok()
            .filter(|p| p.len() <= 4096)
            .map_or(0, |p| {
                availability::status(std::path::Path::new(p)) as c_int
            })
    })
    .unwrap_or(0)
}

/// # Safety
/// `buffer` points to at least `capacity` writable bytes. Returns bytes including NUL, or zero.
#[no_mangle]
pub unsafe extern "C" fn neutrondrive_sync_root(buffer: *mut c_char, capacity: usize) -> usize {
    std::panic::catch_unwind(|| {
        let Some(root) = paths::configured_root() else {
            return 0;
        };
        let Some(path) = root.to_str() else { return 0 };
        let Ok(text) = CString::new(path) else {
            return 0;
        };
        let bytes = text.as_bytes_with_nul();
        if buffer.is_null() || bytes.len() > capacity {
            return 0;
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), buffer.cast(), bytes.len());
        bytes.len()
    })
    .unwrap_or(0)
}
