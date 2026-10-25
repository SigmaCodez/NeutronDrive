//! Optional Nemo 6.x sidebar adapter. Use only the inspected model type and
//! column types; keep all GTK access on the host main loop and never query files.
use super::*;
use std::sync::Mutex;
#[repr(C)]
#[derive(Clone, Copy)]
struct Iter {
    stamp: c_int,
    data: [Ptr; 3],
}
struct Api {
    windows: unsafe extern "C" fn() -> *mut List,
    children: unsafe extern "C" fn(Ptr) -> *mut List,
    container_type: unsafe extern "C" fn() -> GType,
    view_type: unsafe extern "C" fn() -> GType,
    is_a: unsafe extern "C" fn(Ptr, GType) -> c_int,
    name: unsafe extern "C" fn(Ptr) -> *const c_char,
    model: unsafe extern "C" fn(Ptr) -> Ptr,
    filter_child: unsafe extern "C" fn(Ptr) -> Ptr,
    columns: unsafe extern "C" fn(Ptr) -> c_int,
    column_type: unsafe extern "C" fn(Ptr, c_int) -> GType,
    icon_type: unsafe extern "C" fn() -> GType,
    iter_children: unsafe extern "C" fn(Ptr, *mut Iter, *const Iter) -> c_int,
    iter_next: unsafe extern "C" fn(Ptr, *mut Iter) -> c_int,
    get: unsafe extern "C" fn(Ptr, *const Iter, ...),
    set: unsafe extern "C" fn(Ptr, *const Iter, ...),
    path: unsafe extern "C" fn(Ptr, *const Iter) -> Ptr,
    path_free: unsafe extern "C" fn(Ptr),
    path_compare: unsafe extern "C" fn(Ptr, Ptr) -> c_int,
    row_new: unsafe extern "C" fn(Ptr, Ptr) -> Ptr,
    row_path: unsafe extern "C" fn(Ptr) -> Ptr,
    row_free: unsafe extern "C" fn(Ptr),
    get_iter: unsafe extern "C" fn(Ptr, *mut Iter, Ptr) -> c_int,
    icon: unsafe extern "C" fn(*const c_char) -> Ptr,
    equal: unsafe extern "C" fn(Ptr, Ptr) -> c_int,
    filename: unsafe extern "C" fn(*const c_char, *mut *mut c_char, Ptr) -> *mut c_char,
    reference: unsafe extern "C" fn(Ptr) -> Ptr,
    unref: unsafe extern "C" fn(Ptr),
    free: unsafe extern "C" fn(Ptr),
    list_free: unsafe extern "C" fn(*mut List),
}
struct Saved {
    model: usize,
    row: usize,
    icon: usize,
    seen: bool,
}
static API: OnceLock<Option<Api>> = OnceLock::new();
static SAVED: Mutex<Vec<Saved>> = Mutex::new(Vec::new());
unsafe fn load() -> Option<Api> {
    Some(Api {
        windows: symbol("gtk_window_list_toplevels")?,
        children: symbol("gtk_container_get_children")?,
        container_type: symbol("gtk_container_get_type")?,
        view_type: symbol("gtk_tree_view_get_type")?,
        is_a: symbol("g_type_check_instance_is_a")?,
        name: symbol("g_type_name_from_instance")?,
        model: symbol("gtk_tree_view_get_model")?,
        filter_child: symbol("gtk_tree_model_filter_get_model")?,
        columns: symbol("gtk_tree_model_get_n_columns")?,
        column_type: symbol("gtk_tree_model_get_column_type")?,
        icon_type: symbol("g_icon_get_type")?,
        iter_children: symbol("gtk_tree_model_iter_children")?,
        iter_next: symbol("gtk_tree_model_iter_next")?,
        get: symbol("gtk_tree_model_get")?,
        set: symbol("gtk_tree_store_set")?,
        path: symbol("gtk_tree_model_get_path")?,
        path_free: symbol("gtk_tree_path_free")?,
        path_compare: symbol("gtk_tree_path_compare")?,
        row_new: symbol("gtk_tree_row_reference_new")?,
        row_path: symbol("gtk_tree_row_reference_get_path")?,
        row_free: symbol("gtk_tree_row_reference_free")?,
        get_iter: symbol("gtk_tree_model_get_iter")?,
        icon: symbol("g_themed_icon_new")?,
        equal: symbol("g_icon_equal")?,
        filename: symbol("g_filename_from_uri")?,
        reference: symbol("g_object_ref")?,
        unref: symbol("g_object_unref")?,
        free: symbol("g_free")?,
        list_free: symbol("g_list_free")?,
    })
}
unsafe fn named(api: &Api, object: Ptr, name: &[u8]) -> bool {
    !object.is_null() && CStr::from_ptr((api.name)(object)).to_bytes() == name
}
unsafe fn target(api: &Api, uri: *const c_char, root: Option<&std::path::Path>) -> bool {
    if uri.is_null() {
        return false;
    }
    let mut host = ptr::null_mut();
    let path = (api.filename)(uri, &mut host, ptr::null_mut());
    let local = host.is_null() || CStr::from_ptr(host).to_bytes().is_empty();
    if !host.is_null() {
        (api.free)(host.cast())
    }
    let result = local
        && !path.is_null()
        && CStr::from_ptr(path)
            .to_str()
            .ok()
            .is_some_and(|p| Some(std::path::Path::new(p)) == root);
    if !path.is_null() {
        (api.free)(path.cast())
    }
    result
}
unsafe fn decorate(api: &Api, model: Ptr, root: Option<&std::path::Path>, saved: &mut Vec<Saved>) {
    if !named(api, model, b"NemoShortcutsModel")
        || (api.columns)(model) != 18
        || (api.column_type)(model, 1) != 16 << 2
        || (api.column_type)(model, 6) != (api.icon_type)()
    {
        return;
    }
    let mut iter = Iter {
        stamp: 0,
        data: [ptr::null_mut(); 3],
    };
    let mut stack = Vec::new();
    if (api.iter_children)(model, &mut iter, ptr::null()) != 0 {
        stack.push((iter, 0))
    }
    let mut visited = 0;
    while let Some((mut iter, depth)) = stack.pop() {
        visited += 1;
        if visited > 4096 {
            break;
        }
        let mut uri: *mut c_char = ptr::null_mut();
        (api.get)(model, &iter, 1, &mut uri, -1);
        let matches = target(api, uri, root);
        if !uri.is_null() {
            (api.free)(uri.cast())
        }
        if matches {
            let path = (api.path)(model, &iter);
            let mut found = false;
            for entry in saved.iter_mut().filter(|e| e.model == model as usize) {
                let old_path = (api.row_path)(entry.row as Ptr);
                if !old_path.is_null() {
                    if (api.path_compare)(old_path, path) == 0 {
                        entry.seen = true;
                        found = true
                    }
                    (api.path_free)(old_path)
                }
            }
            let mut old: Ptr = ptr::null_mut();
            (api.get)(model, &iter, 6, &mut old, -1);
            let icon = (api.icon)(c"io.github.SigmaCodez.NeutronDrive".as_ptr());
            if (api.equal)(old, icon) == 0 {
                (api.set)(model, &iter, 6, icon, -1)
            }
            (api.unref)(icon);
            if found {
                if !old.is_null() {
                    (api.unref)(old)
                }
            } else {
                saved.push(Saved {
                    model: (api.reference)(model) as usize,
                    row: (api.row_new)(model, path) as usize,
                    icon: old as usize,
                    seen: true,
                })
            }
            (api.path_free)(path);
        }
        let parent = iter;
        if (api.iter_next)(model, &mut iter) != 0 {
            stack.push((iter, depth))
        }
        if depth < 32 && (api.iter_children)(model, &mut iter, &parent) != 0 {
            stack.push((iter, depth + 1))
        }
    }
}
unsafe fn restore(api: &Api, saved: Saved) {
    let path = (api.row_path)(saved.row as Ptr);
    if !path.is_null() {
        let mut iter = Iter {
            stamp: 0,
            data: [ptr::null_mut(); 3],
        };
        if (api.get_iter)(saved.model as Ptr, &mut iter, path) != 0 {
            (api.set)(saved.model as Ptr, &iter, 6, saved.icon as Ptr, -1)
        }
        (api.path_free)(path)
    }
    (api.row_free)(saved.row as Ptr);
    if saved.icon != 0 {
        (api.unref)(saved.icon as Ptr)
    }
    (api.unref)(saved.model as Ptr);
}
pub unsafe fn refresh() {
    if !super::API.get().is_some_and(|a| a.host == Host::Nemo) {
        return;
    }
    let Some(api) = API.get_or_init(|| load()).as_ref() else {
        return;
    };
    let root = paths::configured_root();
    let mut saved = SAVED.lock().unwrap();
    for entry in saved.iter_mut() {
        entry.seen = false
    }
    let windows = (api.windows)();
    let mut item = windows;
    let mut stack = Vec::new();
    while !item.is_null() && stack.len() < 64 {
        stack.push(((*item).data, 0));
        item = (*item).next
    }
    (api.list_free)(windows);
    let mut visited = 0;
    while let Some((widget, depth)) = stack.pop() {
        visited += 1;
        if visited > 8192 {
            break;
        }
        if (api.is_a)(widget, (api.view_type)()) != 0 {
            let mut model = (api.model)(widget);
            if named(api, model, b"GtkTreeModelFilter") {
                model = (api.filter_child)(model)
            }
            decorate(api, model, root.as_deref(), &mut saved)
        }
        if depth < 40 && (api.is_a)(widget, (api.container_type)()) != 0 {
            let children = (api.children)(widget);
            let mut child = children;
            while !child.is_null() && stack.len() < 8192 {
                stack.push(((*child).data, depth + 1));
                child = (*child).next
            }
            (api.list_free)(children)
        }
    }
    let mut retained = Vec::new();
    for entry in saved.drain(..) {
        if entry.seen {
            retained.push(entry)
        } else {
            restore(api, entry)
        }
    }
    *saved = retained;
}
pub unsafe fn shutdown() {
    if let Some(Some(api)) = API.get() {
        for entry in SAVED.lock().unwrap().drain(..) {
            restore(api, entry)
        }
    }
}
