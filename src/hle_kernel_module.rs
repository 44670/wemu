// HMODULE GetModuleHandleA(LPCSTR name)
// Resolve a loaded fake module handle by name.
fn hle_get_module_handle_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let name = arg(emu, 0);
    if name == 0 {
        let image_base = emu.image.as_ref().map(|i| i.image_base).unwrap_or(0x400000);
        ret(emu, image_base);
        return HleResult::Retn(4);
    }
    let s = emu.memory.cstr_lossy(name, 260).hle();
    let h = module_handle_by_name(emu, &s).unwrap_or(0);
    if emu.should_trace() {
        eprintln!("GetModuleHandleA name={s:?} -> {h:08x}");
    }
    ret(emu, h);
    HleResult::Retn(4)
}

// HMODULE LoadLibraryA(LPCSTR name)
// Load a PE DLL image when available, otherwise return a fake module handle.
fn hle_load_library_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let name_addr = arg(emu, 0);
    let name = emu.memory.cstr_lossy(name_addr, 260).hle();
    let h = load_library_common(emu, &name);
    ret(emu, h);
    HleResult::Retn(4)
}

// HMODULE LoadLibraryExA(LPCSTR name, HANDLE file, DWORD flags)
// Load a PE DLL image like LoadLibraryA while ignoring unsupported flags.
fn hle_load_library_ex_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let name_addr = arg(emu, 0);
    let name = emu.memory.cstr_lossy(name_addr, 260).hle();
    let h = load_library_common(emu, &name);
    ret(emu, h);
    HleResult::Retn(12)
}

// HINSTANCE LoadModule(LPCSTR name, LPVOID parameter_block)
// Accept legacy guest-side launch requests without spawning host processes.
fn hle_load_module(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 33);
    HleResult::Retn(8)
}

// BOOL FreeLibrary(HMODULE mod)
// Accept library release without unloading HLE symbols.
fn hle_free_library(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(4)
}

// FARPROC GetProcAddress(HMODULE mod, LPCSTR name)
// Resolve mapped PE exports first, otherwise return an HLE thunk address.
fn hle_get_proc_address(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let module = arg(emu, 0);
    let name_addr = arg(emu, 1);
    let name = if name_addr < 0x10000 {
        format!("#{name_addr}")
    } else {
        emu.memory.cstr_lossy(name_addr, 260).hle()
    };
    if let Some(addr) = emu
        .hle
        .module_images
        .get(&module)
        .and_then(|image| image.resolve_export(&name))
    {
        ret(emu, addr);
        return HleResult::Retn(8);
    }
    if let Some(addr) = emu
        .image
        .as_ref()
        .filter(|image| image.image_base == module)
        .and_then(|image| image.resolve_export(&name))
    {
        ret(emu, addr);
        return HleResult::Retn(8);
    }
    if emu.hle.module_images.contains_key(&module) {
        emu.hle.last_error = 127;
        ret(emu, 0);
        return HleResult::Retn(8);
    }
    if let Some(dll) = emu.hle.hle_runtime_module_for_handle(module) {
        let addr = emu.hle.resolve_import(&dll, &name);
        if emu.should_trace() {
            eprintln!("GetProcAddress module={module:08x} dll={dll} name={name:?} -> {addr:08x}");
        }
        ret(emu, addr);
    } else {
        if emu.should_trace() {
            eprintln!("GetProcAddress module={module:08x} name={name:?} -> 00000000");
        }
        emu.hle.last_error = 127;
        ret(emu, 0);
    }
    HleResult::Retn(8)
}

// BOOL GetModuleHandleExA(DWORD flags, LPCSTR name, HMODULE *module)
// Resolve a loaded module by ANSI name or containing address.
fn hle_get_module_handle_ex_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let flags = arg(emu, 0);
    let name = arg(emu, 1);
    let out = arg(emu, 2);
    let handle = get_module_handle_ex_impl(emu, flags, name, false);
    if let Some(handle) = handle {
        if out != 0 {
            emu.memory.write_u32(out, handle).hle();
        }
        ret(emu, 1);
    } else {
        emu.hle.last_error = 126;
        ret(emu, 0);
    }
    HleResult::Retn(12)
}

// BOOL DestroyIcon(HICON icon)
// Accept release of fake resource icon handles.
fn hle_destroy_icon(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 1);
    HleResult::Retn(4)
}

// HBITMAP LoadBitmapA(HINSTANCE inst, LPCSTR name)
// Load an ANSI-named or integer RT_BITMAP DIB into a surface-backed HBITMAP.
fn hle_load_bitmap_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let module = arg(emu, 0);
    let name = arg(emu, 1);
    let handle = read_resource_key(emu, name, false)
        .as_ref()
        .and_then(|key| load_bitmap_resource_by_key(emu, module, key))
        .unwrap_or(0);
    trace_gdi!("gdi LoadBitmapA module={module:08x} name={name:08x} -> {handle:08x}");
    ret(emu, handle);
    HleResult::Retn(8)
}

// HBITMAP LoadBitmapW(HINSTANCE inst, LPCWSTR name)
// Load a wide-named or integer RT_BITMAP DIB into a surface-backed HBITMAP.
fn hle_load_bitmap_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let module = arg(emu, 0);
    let name = arg(emu, 1);
    let handle = read_resource_key(emu, name, true)
        .as_ref()
        .and_then(|key| load_bitmap_resource_by_key(emu, module, key))
        .unwrap_or(0);
    trace_gdi!("gdi LoadBitmapW module={module:08x} name={name:08x} -> {handle:08x}");
    ret(emu, handle);
    HleResult::Retn(8)
}

// HANDLE LoadImageA(HINSTANCE inst, LPCSTR name, UINT type, int cx, int cy, UINT flags)
// Load bitmap resources through the bitmap path and return stable icon/cursor handles.
fn hle_load_image_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    const IMAGE_BITMAP: u32 = 0;

    let module = arg(emu, 0);
    let name = arg(emu, 1);
    let image_type = arg(emu, 2);
    let handle = if image_type == IMAGE_BITMAP {
        read_resource_key(emu, name, false)
            .as_ref()
            .and_then(|key| load_bitmap_resource_by_key(emu, module, key))
            .unwrap_or(0)
    } else {
        0x5301_0000 | ((image_type & 0xff) << 16) | (name & 0xffff)
    };
    ret(emu, handle);
    HleResult::Retn(24)
}

// HANDLE LoadImageW(HINSTANCE inst, LPCWSTR name, UINT type, int cx, int cy, UINT flags)
// Load bitmap resources through the bitmap path and return stable icon/cursor handles.
fn hle_load_image_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    const IMAGE_BITMAP: u32 = 0;

    let module = arg(emu, 0);
    let name = arg(emu, 1);
    let image_type = arg(emu, 2);
    let handle = if image_type == IMAGE_BITMAP {
        read_resource_key(emu, name, true)
            .as_ref()
            .and_then(|key| load_bitmap_resource_by_key(emu, module, key))
            .unwrap_or(0)
    } else {
        0x5301_0000 | ((image_type & 0xff) << 16) | (name & 0xffff)
    };
    ret(emu, handle);
    HleResult::Retn(24)
}

// HRSRC FindResourceA(HMODULE module, LPCSTR name, LPCSTR type)
// Return the PE resource data-entry address for integer or named resources.
fn hle_find_resource_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let module = arg(emu, 0);
    let Some(name) = resource_key_a(emu, arg(emu, 1)) else {
        ret(emu, 0);
        return HleResult::Retn(12);
    };
    let Some(resource_type) = resource_key_a(emu, arg(emu, 2)) else {
        ret(emu, 0);
        return HleResult::Retn(12);
    };
    let handle = find_pe_resource_data_entry(emu, module, &resource_type, &name).unwrap_or(0);
    ret(emu, handle);
    HleResult::Retn(12)
}

// HRSRC FindResourceW(HMODULE module, LPCWSTR name, LPCWSTR type)
// Return the PE resource data-entry address for integer or named resources.
fn hle_find_resource_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let module = arg(emu, 0);
    let Some(name) = resource_key_w(emu, arg(emu, 1)) else {
        ret(emu, 0);
        return HleResult::Retn(12);
    };
    let Some(resource_type) = resource_key_w(emu, arg(emu, 2)) else {
        ret(emu, 0);
        return HleResult::Retn(12);
    };
    let handle = find_pe_resource_data_entry(emu, module, &resource_type, &name).unwrap_or(0);
    ret(emu, handle);
    HleResult::Retn(12)
}

// HGLOBAL LoadResource(HMODULE module, HRSRC resource)
// Convert the HRSRC data-entry handle into the mapped resource data pointer.
fn hle_load_resource(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let module = arg(emu, 0);
    let resource = arg(emu, 1);
    let value = pe_resource_image_base_and_entry(emu, module, resource)
        .and_then(|(image_base, entry)| emu.memory.read_u32(entry).ok().map(|rva| image_base + rva))
        .unwrap_or(0);
    ret(emu, value);
    HleResult::Retn(8)
}

// LPVOID LockResource(HGLOBAL resource)
// Return the direct resource data pointer produced by LoadResource.
fn hle_lock_resource(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, arg(emu, 0));
    HleResult::Retn(4)
}

// DWORD SizeofResource(HMODULE module, HRSRC resource)
// Return the byte size stored in the PE resource data entry.
fn hle_sizeof_resource(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let module = arg(emu, 0);
    let resource = arg(emu, 1);
    let size = pe_resource_image_base_and_entry(emu, module, resource)
        .and_then(|(_, entry)| emu.memory.read_u32(entry + 4).ok())
        .unwrap_or(0);
    ret(emu, size);
    HleResult::Retn(8)
}

// BOOL FreeResource(HGLOBAL resource)
// Match Win32's obsolete no-op FreeResource behavior.
fn hle_free_resource(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0);
    HleResult::Retn(4)
}

// HMODULE GetModuleHandleW(LPCWSTR name)
// Resolve a loaded fake module handle by wide DLL name.
fn hle_get_module_handle_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let name = arg(emu, 0);
    if name == 0 {
        let image_base = emu.image.as_ref().map(|i| i.image_base).unwrap_or(0x400000);
        ret(emu, image_base);
        return HleResult::Retn(4);
    }
    let s = emu.memory.utf16z_lossy(name, 260).hle();
    let h = module_handle_by_name(emu, &s).unwrap_or(0);
    if emu.should_trace() {
        eprintln!("GetModuleHandleW name={s:?} -> {h:08x}");
    }
    ret(emu, h);
    HleResult::Retn(4)
}

// HMODULE LoadLibraryW(LPCWSTR name)
// Load a PE DLL image when available, otherwise return a fake module handle.
fn hle_load_library_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let name_addr = arg(emu, 0);
    let name = emu.memory.utf16z_lossy(name_addr, 260).hle();
    let h = load_library_common(emu, &name);
    ret(emu, h);
    HleResult::Retn(4)
}

// HMODULE LoadLibraryExW(LPCWSTR name, HANDLE file, DWORD flags)
// Load a PE DLL image like LoadLibraryW while ignoring unsupported flags.
fn hle_load_library_ex_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let name_addr = arg(emu, 0);
    let name = emu.memory.utf16z_lossy(name_addr, 260).hle();
    let h = load_library_common(emu, &name);
    ret(emu, h);
    HleResult::Retn(12)
}

// BOOL GetModuleHandleExW(DWORD flags, LPCWSTR name, HMODULE *module)
// Resolve a loaded module by wide name or containing address.
fn hle_get_module_handle_ex_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let flags = arg(emu, 0);
    let name = arg(emu, 1);
    let out = arg(emu, 2);
    let handle = get_module_handle_ex_impl(emu, flags, name, true);
    if let Some(handle) = handle {
        if out != 0 {
            emu.memory.write_u32(out, handle).hle();
        }
        ret(emu, 1);
    } else {
        emu.hle.last_error = 126;
        ret(emu, 0);
    }
    HleResult::Retn(12)
}

// HWND CreateDialogParamW(HINSTANCE inst, LPCWSTR tmpl, HWND parent, DLGPROC proc, LPARAM param)
// Create a tracked resource-backed dialog and synchronously deliver WM_INITDIALOG.
fn hle_create_dialog_param_w(emu: &mut Emulator, entry: &HleEntry) -> HleResult {
    create_dialog_param_common(emu, entry, true)
}

// HWND CreateDialogParamA(HINSTANCE inst, LPCSTR tmpl, HWND parent, DLGPROC proc, LPARAM param)
// Create a tracked resource-backed dialog and synchronously deliver WM_INITDIALOG.
fn hle_create_dialog_param_a(emu: &mut Emulator, entry: &HleEntry) -> HleResult {
    create_dialog_param_common(emu, entry, true)
}

// HMENU LoadMenuA(HINSTANCE inst, LPCSTR name)
// Load a named or integer RT_MENU resource into tracked menu/submenu handles.
fn hle_load_menu_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let module = arg(emu, 0);
    let menu = resource_key_a(emu, arg(emu, 1))
        .and_then(|name| load_menu_resource(emu, module, &name))
        .unwrap_or(0);
    ret(emu, menu);
    HleResult::Retn(8)
}

// HMENU LoadMenuW(HINSTANCE inst, LPCWSTR name)
// Load a named or integer RT_MENU resource into tracked menu/submenu handles.
fn hle_load_menu_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let module = arg(emu, 0);
    let menu = resource_key_w(emu, arg(emu, 1))
        .and_then(|name| load_menu_resource(emu, module, &name))
        .unwrap_or(0);
    ret(emu, menu);
    HleResult::Retn(8)
}

// HACCEL LoadAcceleratorsA(HINSTANCE inst, LPCSTR table)
// Load a named or integer RT_ACCELERATOR resource into a tracked table.
fn hle_load_accelerators_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let module = arg(emu, 0);
    let accel = resource_key_a(emu, arg(emu, 1))
        .and_then(|name| load_accelerator_resource(emu, module, &name))
        .unwrap_or(0);
    ret(emu, accel);
    HleResult::Retn(8)
}

// HACCEL LoadAcceleratorsW(HINSTANCE inst, LPCWSTR table)
// Load a named or integer RT_ACCELERATOR resource into a tracked table.
fn hle_load_accelerators_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let module = arg(emu, 0);
    let accel = resource_key_w(emu, arg(emu, 1))
        .and_then(|name| load_accelerator_resource(emu, module, &name))
        .unwrap_or(0);
    ret(emu, accel);
    HleResult::Retn(8)
}

// int LoadStringW(HINSTANCE inst, UINT id, LPWSTR out, int max)
// Load UTF-16 string-table resources with a Notepad compatibility fallback.
fn hle_load_string_w(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let inst = arg(emu, 0);
    let id = arg(emu, 1);
    let out = arg(emu, 2);
    let max = arg(emu, 3) as usize;
    let value = load_string_resource(emu, inst, id)
        .or_else(|| notepad_string_resource(id).map(str::to_string))
        .unwrap_or_default();
    let len = value.encode_utf16().count();
    if out != 0 && max != 0 {
        emu.memory.write_utf16z(out, &value, max).hle();
    }
    ret(emu, len as u32);
    HleResult::Retn(16)
}

// int LoadStringA(HINSTANCE inst, UINT id, LPSTR out, int max)
// Load string-table resources as ANSI with the same fallback table as LoadStringW.
fn hle_load_string_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let inst = arg(emu, 0);
    let id = arg(emu, 1);
    let out = arg(emu, 2);
    let max = arg(emu, 3) as usize;
    let value = load_string_resource(emu, inst, id)
        .or_else(|| notepad_string_resource(id).map(str::to_string))
        .unwrap_or_default();
    if out != 0 && max != 0 {
        emu.memory.write_cstr(out, &value, max).hle();
    }
    ret(emu, value.len() as u32);
    HleResult::Retn(16)
}

// INT_PTR DialogBoxParamA(HINSTANCE inst, LPCSTR tmpl, HWND parent, DLGPROC proc, LPARAM param)
// Create a tracked resource-backed modal dialog and return after WM_INITDIALOG.
fn hle_dialog_box_param_a(emu: &mut Emulator, entry: &HleEntry) -> HleResult {
    create_dialog_param_common(emu, entry, false)
}

fn load_library_common(emu: &mut Emulator, name: &str) -> u32 {
    if let Some(h) = module_handle_by_name(emu, name) {
        return h;
    }
    for load_name in module_load_names(name) {
        let Ok(path) = emu.hle.translate_raw_path(&load_name) else {
            continue;
        };
        let path = if path.is_file() {
            Some(path)
        } else {
            case_insensitive_existing_path(&path).filter(|candidate| candidate.is_file())
        };
        if let Some(path) = path {
            match crate::pe::load_pe32_dll(&path, &mut emu.memory, &mut emu.hle)
                .or_else(|_| crate::pe::load_pe32_resource_image(&path, &mut emu.memory, &mut emu.hle))
            {
                Ok(image) => {
                    let h = image.image_base;
                    emu.hle.module_images.insert(h, image);
                    register_module_aliases(emu, h, name);
                    register_module_aliases(emu, h, &load_name);
                    if let Some(file_name) = path.file_name().and_then(|s| s.to_str()) {
                        register_module_aliases(emu, h, file_name);
                    }
                    emu.hle
                        .check_strict_hle_imports()
                        .unwrap_or_else(|err| panic!("{err}"));
                    return h;
                }
                Err(err) if emu.should_trace() => {
                    eprintln!("LoadLibrary resource image {path:?} failed: {err}");
                }
                Err(_) => {}
            }
        }
    }
    let Some(runtime_name) = module_lookup_keys(name)
        .into_iter()
        .find(|key| is_hle_runtime_dll(key))
    else {
        emu.hle.last_error = 126;
        return 0;
    };
    let h = emu.hle.alloc_module();
    register_module_aliases(emu, h, name);
    register_module_aliases(emu, h, &runtime_name);
    h
}

fn get_module_handle_ex_impl(emu: &mut Emulator, flags: u32, name: u32, wide: bool) -> Option<u32> {
    const GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS: u32 = 0x0000_0004;

    if (flags & GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS) != 0 {
        let addr = name;
        if let Some(image) = emu.image.as_ref() {
            let end = image.image_base.wrapping_add(image.size_of_image);
            if addr >= image.image_base && addr < end {
                return Some(image.image_base);
            }
        }
        return emu
            .hle
            .module_images
            .values()
            .find(|image| {
                let end = image.image_base.wrapping_add(image.size_of_image);
                addr >= image.image_base && addr < end
            })
            .map(|image| image.image_base);
    }
    if name == 0 {
        return emu.image.as_ref().map(|image| image.image_base);
    }
    let name = if wide {
        emu.memory.utf16z_lossy(name, 260).ok()?
    } else {
        emu.memory.cstr_lossy(name, 260).ok()?
    };
    module_handle_by_name(emu, &name)
}

fn module_handle_by_name(emu: &Emulator, name: &str) -> Option<u32> {
    module_lookup_keys(name)
        .into_iter()
        .find_map(|key| emu.hle.modules.get(&key).copied())
}

fn register_module_aliases(emu: &mut Emulator, handle: u32, name: &str) {
    for key in module_lookup_keys(name) {
        emu.hle.modules.insert(key, handle);
    }
}

fn module_load_names(name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let normalized = normalize_module_name(name);
    push_unique_string(&mut out, normalized.clone());
    if module_name_needs_default_dll_extension(&normalized) {
        push_unique_string(&mut out, format!("{normalized}.dll"));
    }
    out
}

fn module_lookup_keys(name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let normalized = normalize_module_name(name);
    push_unique_string(&mut out, normalized.clone());
    if module_name_needs_default_dll_extension(&normalized) {
        push_unique_string(&mut out, format!("{normalized}.dll"));
    }
    let base = normalized
        .rsplit('\\')
        .next()
        .unwrap_or(normalized.as_str())
        .to_string();
    push_unique_string(&mut out, base.clone());
    if module_name_needs_default_dll_extension(&base) {
        push_unique_string(&mut out, format!("{base}.dll"));
    }
    out
}

fn normalize_module_name(name: &str) -> String {
    name.trim()
        .trim_matches('\0')
        .replace('/', "\\")
        .to_ascii_lowercase()
}

fn module_name_needs_default_dll_extension(name: &str) -> bool {
    let base = name.rsplit('\\').next().unwrap_or(name);
    !base.is_empty() && !base.contains('.') && !base.ends_with('.')
}

fn push_unique_string(out: &mut Vec<String>, value: String) {
    if !value.is_empty() && !out.iter().any(|old| old == &value) {
        out.push(value);
    }
}

fn make_int_resource_id(ptr: u32) -> Option<u32> {
    (ptr != 0 && ptr < 0x0001_0000).then_some(ptr & 0xffff)
}

fn resource_key_a(emu: &Emulator, ptr: u32) -> Option<ResourceKey> {
    if let Some(id) = make_int_resource_id(ptr) {
        return Some(ResourceKey::Id(id));
    }
    if ptr == 0 {
        return None;
    }
    emu.memory
        .cstr_lossy(ptr, 256)
        .ok()
        .map(ResourceKey::Name)
}

fn resource_key_w(emu: &Emulator, ptr: u32) -> Option<ResourceKey> {
    if let Some(id) = make_int_resource_id(ptr) {
        return Some(ResourceKey::Id(id));
    }
    if ptr == 0 {
        return None;
    }
    emu.memory
        .utf16z_lossy(ptr, 256)
        .ok()
        .map(ResourceKey::Name)
}

fn read_resource_key(emu: &Emulator, ptr: u32, wide: bool) -> Option<ResourceKey> {
    if wide {
        resource_key_w(emu, ptr)
    } else {
        resource_key_a(emu, ptr)
    }
}

fn find_pe_resource_data(
    emu: &Emulator,
    module: u32,
    resource_type: u32,
    resource_id: u32,
) -> Option<(u32, u32)> {
    find_pe_resource_data_by_key(emu, module, resource_type, &ResourceKey::Id(resource_id))
}

fn find_pe_resource_data_by_key(
    emu: &Emulator,
    module: u32,
    resource_type: u32,
    resource_name: &ResourceKey,
) -> Option<(u32, u32)> {
    let entry = find_pe_resource_data_entry(
        emu,
        module,
        &ResourceKey::Id(resource_type),
        resource_name,
    )?;
    let (image_base, data_entry) = pe_resource_image_base_and_entry(emu, module, entry)?;
    let rva = emu.memory.read_u32(data_entry).ok()?;
    let size = emu.memory.read_u32(data_entry + 4).ok()?;
    Some((image_base.wrapping_add(rva), size))
}

fn find_pe_resource_data_entry(
    emu: &Emulator,
    module: u32,
    resource_type: &ResourceKey,
    resource_name: &ResourceKey,
) -> Option<u32> {
    let main_image = emu.image.as_ref()?;
    let image = if module == 0 || module == main_image.image_base {
        main_image
    } else {
        emu.hle.module_images.get(&module)?
    };
    let rsrc = image.sections.iter().find(|section| section.name == ".rsrc")?;
    let root = rsrc.va;
    let type_entry = find_resource_entry_by_key(&emu.memory, root, root, resource_type)?;
    if (type_entry & 0x8000_0000) == 0 {
        return None;
    }
    let name_dir = root.wrapping_add(type_entry & 0x7fff_ffff);
    let name_entry = find_resource_entry_by_key(&emu.memory, root, name_dir, resource_name)?;
    if (name_entry & 0x8000_0000) == 0 {
        return None;
    }
    let lang_dir = root.wrapping_add(name_entry & 0x7fff_ffff);
    let data_entry = find_resource_entry_by_id(&emu.memory, root, lang_dir, LANG_ENGLISH_US)
        .or_else(|| first_resource_entry(&emu.memory, root, lang_dir))?;
    let data_addr = if (data_entry & 0x8000_0000) != 0 {
        let data_dir = root.wrapping_add(data_entry & 0x7fff_ffff);
        first_resource_entry(&emu.memory, root, data_dir)?
    } else {
        data_entry
    };
    Some(root.wrapping_add(data_addr & 0x7fff_ffff))
}

fn pe_resource_image_base_and_entry(
    emu: &Emulator,
    module: u32,
    data_entry: u32,
) -> Option<(u32, u32)> {
    let main_image = emu.image.as_ref()?;
    let image = if module == 0 || module == main_image.image_base {
        main_image
    } else {
        emu.hle.module_images.get(&module)?
    };
    let rsrc = image.sections.iter().find(|section| section.name == ".rsrc")?;
    let end = rsrc.va.saturating_add(rsrc.virtual_size.max(rsrc.raw_size));
    (data_entry >= rsrc.va && data_entry.wrapping_add(16) <= end)
        .then_some((image.image_base, data_entry))
}

fn load_menu_resource(emu: &mut Emulator, module: u32, name: &ResourceKey) -> Option<u32> {
    let entry = find_pe_resource_data_entry(emu, module, &ResourceKey::Id(RT_MENU), name)?;
    let (image_base, data_entry) = pe_resource_image_base_and_entry(emu, module, entry)?;
    let rva = emu.memory.read_u32(data_entry).ok()?;
    let size = emu.memory.read_u32(data_entry + 4).ok()?;
    let addr = image_base.wrapping_add(rva);
    parse_standard_menu_template(emu, addr, size)
}

fn load_accelerator_resource(emu: &mut Emulator, module: u32, name: &ResourceKey) -> Option<u32> {
    let entry =
        find_pe_resource_data_entry(emu, module, &ResourceKey::Id(RT_ACCELERATOR), name)?;
    let (image_base, data_entry) = pe_resource_image_base_and_entry(emu, module, entry)?;
    let rva = emu.memory.read_u32(data_entry).ok()?;
    let size = emu.memory.read_u32(data_entry + 4).ok()?;
    let addr = image_base.wrapping_add(rva);
    parse_accelerator_table(emu, addr, size)
        .map(|table| emu.hle.alloc_accelerator_handle(table))
}

fn find_resource_entry_by_id(mem: &Memory, root: u32, dir: u32, id: u32) -> Option<u32> {
    let named = mem.read_u16(dir + 12).ok()? as u32;
    let ids = mem.read_u16(dir + 14).ok()? as u32;
    for index in 0..named.saturating_add(ids) {
        let entry = dir.wrapping_add(16 + index * 8);
        let name = mem.read_u32(entry).ok()?;
        if (name & 0x8000_0000) == 0 && (name & 0xffff) == id {
            return mem.read_u32(entry + 4).ok();
        }
    }
    let _ = root;
    None
}

fn find_resource_entry_by_key(
    mem: &Memory,
    root: u32,
    dir: u32,
    key: &ResourceKey,
) -> Option<u32> {
    match key {
        ResourceKey::Id(id) => find_resource_entry_by_id(mem, root, dir, *id),
        ResourceKey::Name(name) => find_resource_entry_by_name(mem, root, dir, name),
    }
}

fn find_resource_entry_by_name(mem: &Memory, root: u32, dir: u32, target: &str) -> Option<u32> {
    let named = mem.read_u16(dir + 12).ok()? as u32;
    for index in 0..named {
        let entry = dir.wrapping_add(16 + index * 8);
        let name_ref = mem.read_u32(entry).ok()?;
        if (name_ref & 0x8000_0000) == 0 {
            continue;
        }
        let name = read_resource_dir_string(mem, root.wrapping_add(name_ref & 0x7fff_ffff))?;
        if name.eq_ignore_ascii_case(target) {
            return mem.read_u32(entry + 4).ok();
        }
    }
    None
}

fn read_resource_dir_string(mem: &Memory, addr: u32) -> Option<String> {
    let len = mem.read_u16(addr).ok()? as usize;
    let mut units = Vec::with_capacity(len);
    for index in 0..len {
        units.push(mem.read_u16(addr + 2 + index as u32 * 2).ok()?);
    }
    Some(String::from_utf16_lossy(&units))
}

fn first_resource_entry(mem: &Memory, _root: u32, dir: u32) -> Option<u32> {
    let named = mem.read_u16(dir + 12).ok()? as u32;
    let ids = mem.read_u16(dir + 14).ok()? as u32;
    if named.saturating_add(ids) == 0 {
        return None;
    }
    mem.read_u32(dir + 20).ok()
}

fn load_string_resource(emu: &Emulator, module: u32, id: u32) -> Option<String> {
    let block = id / 16 + 1;
    let index = id % 16;
    let (addr, size) = find_pe_resource_data(emu, module, RT_STRING, block)?;
    let mut r = TemplateReader::new(addr, size);
    for current in 0..16 {
        let len = r.read_u16(&emu.memory)? as usize;
        let string_addr = r.addr()?;
        if current == index {
            let mut units = Vec::with_capacity(len);
            for i in 0..len {
                units.push(emu.memory.read_u16(string_addr + i as u32 * 2).ok()?);
            }
            return Some(String::from_utf16_lossy(&units));
        }
        r.skip((len * 2) as u32)?;
    }
    None
}

fn load_bitmap_resource_by_key(
    emu: &mut Emulator,
    module: u32,
    key: &ResourceKey,
) -> Option<u32> {
    let (addr, size) = find_pe_resource_data_by_key(emu, module, RT_BITMAP, key)?;
    let surface = create_surface_from_dib(emu, addr, size)?;
    let handle = emu.hle.create_gdi_bitmap(surface);
    if hle_trace_enabled(HLE_TRACE_GDI) {
        if let Ok(info) = read_surface_info(emu, surface) {
            eprintln!(
                "gdi bitmap-resource module={module:08x} key={key:?} addr={addr:08x} size={size} surf={surface:08x} {}x{}x{} -> {handle:08x}",
                info.width,
                info.height,
                info.bpp,
            );
        }
    }
    Some(handle)
}

fn notepad_string_resource(id: u32) -> Option<&'static str> {
    match id {
        0x160 => Some("&f"),
        0x161 => Some("Page &p"),
        0x170 => Some("Notepad"),
        0x171 => Some("ERROR"),
        0x172 => Some("WARNING"),
        0x173 => Some("Information"),
        0x174 => Some("Untitled"),
        0x175 => Some("All files (*.*)"),
        0x176 => Some("Text files (*.txt)"),
        0x17b => Some("'%s' can not be found."),
        0x17c => Some("Not enough memory to complete this task."),
        0x17d => Some("Cannot find '%s'"),
        0x180 => Some("ANSI"),
        0x181 => Some("Unicode"),
        0x182 => Some("Unicode (big endian)"),
        0x183 => Some("UTF-8"),
        0x184 => Some("UTF-8 with BOM"),
        0x185 => Some("Windows (CR + LF)"),
        0x186 => Some("Unix (LF)"),
        0x187 => Some("Mac (CR)"),
        0x188 => Some("Line %d, column %d"),
        0x18a => Some("Lucida Console"),
        0x18b => Some("The specified line number is out of range."),
        0x18c => Some("Now printing page..."),
        0x18d => Some("The print job is being canceled..."),
        0x18e => Some("Printing is successfully done."),
        0x18f => Some("Printing has been canceled."),
        0x190 => Some("Printing failed."),
        0x200 => Some("Text Document"),
        0x300 => Some("Copyright 1997,98 Marcel Baur, 2000 Mike McCormack, 2002 Sylvain Petreolle, 2002 Andriy Palamarchuk\r\n"),
        _ => None,
    }
}
