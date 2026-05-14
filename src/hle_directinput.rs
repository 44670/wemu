const DINPUT_KIND_UNKNOWN: u32 = 0;
const DINPUT_KIND_MOUSE: u32 = 1;
const DINPUT_KIND_KEYBOARD: u32 = 2;

const GUID_DATA1_SYSMOUSE: u32 = 0x6f1d_2b60;
const GUID_DATA1_SYSKEYBOARD: u32 = 0x6f1d_2b61;
const GUID_DATA1_SYSMOUSE_EM: u32 = 0x6f1d_2b80;
const GUID_DATA1_SYSMOUSE_EM2: u32 = 0x6f1d_2b81;
const GUID_DATA1_SYSKEYBOARD_EM: u32 = 0x6f1d_2b82;
const GUID_DATA1_SYSKEYBOARD_EM2: u32 = 0x6f1d_2b83;

const DIDEVTYPE_MOUSE: u32 = 0x12;
const DIDEVTYPE_KEYBOARD: u32 = 0x13;

// HRESULT DirectInputCreateA/W(HINSTANCE hinst, DWORD version, IDirectInputA/W **out, IUnknown *outer)
// Create a fake DirectInput root COM object for public DirectX input runtime imports.
fn hle_direct_input_create(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    emu.hle.ensure_dinput_tables(&mut emu.memory).hle();
    let out = arg(emu, 2);
    let obj = create_dinput_object(emu).hle();
    if out != 0 {
        emu.memory.write_u32(out, obj).hle();
    }
    ret(emu, 0);
    HleResult::Retn(16)
}

// HRESULT IDirectInput::CreateDevice(this, REFGUID guid, IDirectInputDeviceA **out, IUnknown *outer)
// Return a fake keyboard/mouse device object; unsupported GUIDs get a neutral generic device.
fn hle_dinput_create_device(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let guid = arg(emu, 1);
    let out = arg(emu, 2);
    let kind = dinput_device_kind(emu, guid);
    let obj = create_dinput_device_object(emu, kind).hle();
    if out != 0 {
        emu.memory.write_u32(out, obj).hle();
    }
    ret(emu, 0);
    HleResult::Retn(16)
}

// HRESULT IDirectInput::EnumDevices(this, DWORD type, CALLBACK cb, void *ref, DWORD flags)
// Report an empty enumeration; games can still open system devices by GUID.
fn hle_dinput_enum_devices(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0);
    HleResult::Retn(20)
}

// HRESULT IDirectInput::GetDeviceStatus(this, REFGUID guid)
// Treat system keyboard/mouse and generic devices as attached.
fn hle_dinput_get_device_status(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, 0);
    HleResult::Retn(8)
}

// HRESULT IDirectInputDevice::GetCapabilities(this, DIDEVCAPS *caps)
// Fill basic attached-device capabilities for keyboard and mouse devices.
fn hle_dinput_device_get_capabilities(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let this = arg(emu, 0);
    let caps = arg(emu, 1);
    if caps != 0 {
        let size = emu.memory.read_u32(caps).unwrap_or(24).max(24);
        let kind = emu.memory.read_u32(this + 8).unwrap_or(DINPUT_KIND_UNKNOWN);
        let (dev_type, axes, buttons) = match kind {
            DINPUT_KIND_MOUSE => (DIDEVTYPE_MOUSE, 3, 4),
            DINPUT_KIND_KEYBOARD => (DIDEVTYPE_KEYBOARD, 0, 256),
            _ => (0, 0, 0),
        };
        let bytes = size.min(44);
        for off in (0..bytes).step_by(4) {
            emu.memory.write_u32(caps + off, 0).hle();
        }
        emu.memory.write_u32(caps, size).hle();
        emu.memory.write_u32(caps + 4, 0x0000_0001).hle();
        emu.memory.write_u32(caps + 8, dev_type).hle();
        emu.memory.write_u32(caps + 12, axes).hle();
        emu.memory.write_u32(caps + 16, buttons).hle();
    }
    ret(emu, 0);
    HleResult::Retn(8)
}

// HRESULT IDirectInputDevice::Acquire(this)
// Mark the fake device acquired so polling APIs can succeed.
fn hle_dinput_device_acquire(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let this = arg(emu, 0);
    if this != 0 {
        emu.memory.write_u32(this + 12, 1).hle();
    }
    ret(emu, 0);
    HleResult::Retn(4)
}

// HRESULT IDirectInputDevice::Unacquire(this)
// Mark the fake device unacquired while keeping the object alive.
fn hle_dinput_device_unacquire(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let this = arg(emu, 0);
    if this != 0 {
        emu.memory.write_u32(this + 12, 0).hle();
    }
    ret(emu, 0);
    HleResult::Retn(4)
}

// HRESULT IDirectInputDevice::GetDeviceState(this, DWORD cbData, void *out)
// Copy neutral mouse state or current frontend keyboard state in DirectInput scan-code form.
fn hle_dinput_device_get_device_state(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let this = arg(emu, 0);
    let cb_data = arg(emu, 1).min(4096);
    let out = arg(emu, 2);
    if out != 0 {
        let zeros = vec![0; cb_data as usize];
        emu.memory.write_bytes(out, &zeros).hle();
        if emu.memory.read_u32(this + 8).unwrap_or(DINPUT_KIND_UNKNOWN) == DINPUT_KIND_KEYBOARD {
            write_directinput_keyboard_state(emu, out, cb_data);
        }
    }
    ret(emu, 0);
    HleResult::Retn(12)
}

// HRESULT IDirectInputDevice::GetDeviceData(this, DWORD cbObjectData, DIDEVICEOBJECTDATA *data, DWORD *count, DWORD flags)
// Report no buffered events; state polling carries the cheap current-state path.
fn hle_dinput_device_get_device_data(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let count = arg(emu, 3);
    if count != 0 {
        emu.memory.write_u32(count, 0).hle();
    }
    ret(emu, 0);
    HleResult::Retn(20)
}

// HRESULT IDirectInputDevice::GetDeviceInfo(this, DIDEVICEINSTANCEA *info)
// Fill a compact ANSI device name for callers that display or validate devices.
fn hle_dinput_device_get_device_info(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let this = arg(emu, 0);
    let info = arg(emu, 1);
    if info != 0 {
        let size = emu.memory.read_u32(info).unwrap_or(0);
        let kind = emu.memory.read_u32(this + 8).unwrap_or(DINPUT_KIND_UNKNOWN);
        let (dev_type, name) = match kind {
            DINPUT_KIND_MOUSE => (DIDEVTYPE_MOUSE, "Mouse"),
            DINPUT_KIND_KEYBOARD => (DIDEVTYPE_KEYBOARD, "Keyboard"),
            _ => (0, "Input Device"),
        };
        if size >= 52 {
            emu.memory.write_u32(info + 36, dev_type).hle();
        }
        if size >= 300 {
            emu.memory.write_cstr(info + 40, name, 260).hle();
        }
        if size >= 560 {
            emu.memory.write_cstr(info + 300, name, 260).hle();
        }
    }
    ret(emu, 0);
    HleResult::Retn(8)
}

fn create_dinput_object(emu: &mut Emulator) -> Result<u32> {
    emu.hle.ensure_dinput_tables(&mut emu.memory)?;
    let obj = emu
        .hle
        .alloc_private(&mut emu.memory, 8, PagePerm::READ | PagePerm::WRITE)?;
    emu.memory.write_u32(obj, emu.hle.dinput_vtable)?;
    emu.memory.write_u32(obj + 4, 1)?;
    Ok(obj)
}

fn create_dinput_device_object(emu: &mut Emulator, kind: u32) -> Result<u32> {
    emu.hle.ensure_dinput_tables(&mut emu.memory)?;
    let obj = emu
        .hle
        .alloc_private(&mut emu.memory, 16, PagePerm::READ | PagePerm::WRITE)?;
    emu.memory.write_u32(obj, emu.hle.dinput_device_vtable)?;
    emu.memory.write_u32(obj + 4, 1)?;
    emu.memory.write_u32(obj + 8, kind)?;
    emu.memory.write_u32(obj + 12, 0)?;
    Ok(obj)
}

fn dinput_device_kind(emu: &Emulator, guid: u32) -> u32 {
    let data1 = emu.memory.read_u32(guid).unwrap_or(0);
    match data1 {
        GUID_DATA1_SYSMOUSE | GUID_DATA1_SYSMOUSE_EM | GUID_DATA1_SYSMOUSE_EM2 => {
            DINPUT_KIND_MOUSE
        }
        GUID_DATA1_SYSKEYBOARD | GUID_DATA1_SYSKEYBOARD_EM | GUID_DATA1_SYSKEYBOARD_EM2 => {
            DINPUT_KIND_KEYBOARD
        }
        _ => DINPUT_KIND_UNKNOWN,
    }
}

fn write_directinput_keyboard_state(emu: &mut Emulator, out: u32, cb_data: u32) {
    for vk in 0..256u32 {
        if !emu.hle.key_down[vk as usize] {
            continue;
        }
        let scan = vk_to_scan_code(vk);
        if scan != 0 && scan < cb_data {
            emu.memory.write_u8(out + scan, 0x80).hle();
        }
    }
}
