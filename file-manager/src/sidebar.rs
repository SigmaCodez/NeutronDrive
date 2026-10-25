//! Optional Nautilus 50 sidebar decoration. Older/changed widget APIs are a no-op.
//! Uses inspected GObject properties only; never patches GTK or reads cloud files.
use super::*;
use std::sync::Mutex;
struct Api {
    windows: unsafe extern "C" fn() -> Ptr,
    count: unsafe extern "C" fn(Ptr) -> c_uint,
    item: unsafe extern "C" fn(Ptr, c_uint) -> Ptr,
    child: unsafe extern "C" fn(Ptr) -> Ptr,
    next: unsafe extern "C" fn(Ptr) -> Ptr,
    type_name: unsafe extern "C" fn(Ptr) -> *const c_char,
    find_property: unsafe extern "C" fn(Ptr, *const c_char) -> Ptr,
    get: unsafe extern "C" fn(Ptr, *const c_char, ...),
    set: unsafe extern "C" fn(Ptr, *const c_char, ...),
    icon: unsafe extern "C" fn(*const c_char) -> Ptr,
    equal: unsafe extern "C" fn(Ptr, Ptr) -> c_int,
    default_value: unsafe extern "C" fn(Ptr) -> Ptr,
    value_holds: unsafe extern "C" fn(Ptr, GType) -> c_int,
    icon_type: unsafe extern "C" fn() -> GType,
    filename: unsafe extern "C" fn(*const c_char, *mut *mut c_char, Ptr) -> *mut c_char,
    reference: unsafe extern "C" fn(Ptr) -> Ptr,
    unref: unsafe extern "C" fn(Ptr),
    free: unsafe extern "C" fn(Ptr),
}
struct Saved {
    row: usize,
    icon: usize,
}
static API: OnceLock<Option<Api>> = OnceLock::new();
static SAVED: Mutex<Vec<Saved>> = Mutex::new(Vec::new());
unsafe fn load() -> Option<Api> {
    Some(Api {
        windows: symbol("gtk_window_get_toplevels")?,
        count: symbol("g_list_model_get_n_items")?,
        item: symbol("g_list_model_get_item")?,
        child: symbol("gtk_widget_get_first_child")?,
        next: symbol("gtk_widget_get_next_sibling")?,
        type_name: symbol("g_type_name_from_instance")?,
        find_property: symbol("g_object_class_find_property")?,
        get: symbol("g_object_get")?,
        set: symbol("g_object_set")?,
        icon: symbol("g_themed_icon_new")?,
        equal: symbol("g_icon_equal")?,
        default_value: symbol("g_param_spec_get_default_value")?,
        value_holds: symbol("g_type_check_value_holds")?,
        icon_type: symbol("g_icon_get_type")?,
        filename: symbol("g_filename_from_uri")?,
        reference: symbol("g_object_ref")?,
        unref: symbol("g_object_unref")?,
        free: symbol("g_free")?,
    })
}
unsafe fn target(api: &Api, row: Ptr, root: Option<&std::path::Path>) -> bool {
    // GtkWidget's leading GTypeInstance contains its class pointer. Do not
    // request properties on unknown widget classes, or on changed versions.
    let name = (api.type_name)(row);
    if name.is_null() || CStr::from_ptr(name).to_bytes() != b"NautilusSidebarRow" {
        return false;
    }
    let class = *(row as *const Ptr);
    let uri_spec = (api.find_property)(class, c"uri".as_ptr());
    let icon_spec = (api.find_property)(class, c"start-icon".as_ptr());
    // G_TYPE_STRING is the stable fundamental type 16 << 2.
    if uri_spec.is_null()
        || icon_spec.is_null()
        || (api.value_holds)((api.default_value)(uri_spec), 16 << 2) == 0
        || (api.value_holds)((api.default_value)(icon_spec), (api.icon_type)()) == 0
    {
        return false;
    }
    let mut uri: *mut c_char = ptr::null_mut();
    (api.get)(row, c"uri".as_ptr(), &mut uri, ptr::null::<c_char>());
    if uri.is_null() {
        return false;
    }
    let mut hostname: *mut c_char = ptr::null_mut();
    let path = (api.filename)(uri, &mut hostname, ptr::null_mut());
    (api.free)(uri.cast());
    let local = hostname.is_null() || CStr::from_ptr(hostname).to_bytes().is_empty();
    if !hostname.is_null() {
        (api.free)(hostname.cast());
    }
    let same = local
        && !path.is_null()
        && CStr::from_ptr(path)
            .to_str()
            .ok()
            .is_some_and(|p| Some(std::path::Path::new(p)) == root);
    if !path.is_null() {
        (api.free)(path.cast());
    }
    same
}
unsafe fn restore(api: &Api, saved: Saved) {
    (api.set)(
        saved.row as Ptr,
        c"start-icon".as_ptr(),
        saved.icon as Ptr,
        ptr::null::<c_char>(),
    );
    if saved.icon != 0 {
        (api.unref)(saved.icon as Ptr);
    }
    (api.unref)(saved.row as Ptr);
}
pub unsafe fn refresh() {
    if !super::API.get().is_some_and(|a| a.host == Host::Nautilus) {
        return;
    }
    let Some(api) = API.get_or_init(|| load()).as_ref() else {
        return;
    };
    let root = paths::configured_root();
    let model = (api.windows)();
    let mut matching = Vec::new();
    let mut visited = 0;
    for i in 0..(api.count)(model).min(64) {
        let window = (api.item)(model, i);
        if window.is_null() {
            continue;
        }
        let mut stack = vec![(window, 0)];
        while let Some((widget, depth)) = stack.pop() {
            visited += 1;
            if visited > 8192 {
                break;
            }
            if target(api, widget, root.as_deref()) {
                matching.push(widget as usize);
            }
            if depth >= 40 {
                continue;
            }
            let mut child = (api.child)(widget);
            while !child.is_null() && stack.len() < 8192 {
                stack.push((child, depth + 1));
                child = (api.next)(child);
            }
        }
        (api.unref)(window);
    }
    let mut saved = SAVED.lock().unwrap();
    let mut retained = Vec::new();
    for entry in saved.drain(..) {
        if matching.contains(&entry.row) {
            retained.push(entry);
        } else {
            restore(api, entry);
        }
    }
    for row in matching {
        let mut old: Ptr = ptr::null_mut();
        (api.get)(
            row as Ptr,
            c"start-icon".as_ptr(),
            &mut old,
            ptr::null::<c_char>(),
        );
        let icon = (api.icon)(c"io.github.SigmaCodez.NeutronDrive".as_ptr());
        if (api.equal)(old, icon) == 0 {
            (api.set)(
                row as Ptr,
                c"start-icon".as_ptr(),
                icon,
                ptr::null::<c_char>(),
            );
        }
        (api.unref)(icon);
        if retained.iter().any(|s| s.row == row) {
            if !old.is_null() {
                (api.unref)(old);
            }
            continue;
        }
        retained.push(Saved {
            row: (api.reference)(row as Ptr) as usize,
            icon: old as usize,
        });
    }
    *saved = retained;
}
pub unsafe fn shutdown() {
    if let Some(Some(api)) = API.get() {
        for saved in SAVED.lock().unwrap().drain(..) {
            restore(api, saved);
        }
    }
}
