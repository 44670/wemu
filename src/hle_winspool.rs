const ERROR_INVALID_HANDLE: u32 = 6;
const ERROR_INVALID_PARAMETER: u32 = 87;
const DEVMODEA_SIZE: usize = 156;
const DM_OUT_BUFFER: u32 = 0x0000_0002;
const IDOK: u32 = 1;

// BOOL OpenPrinterA(LPSTR pPrinterName, LPHANDLE phPrinter, LPPRINTER_DEFAULTSA pDefault)
// Return a lightweight printer handle for applications that probe print support.
fn hle_open_printer_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let printer_name = arg(emu, 0);
    let out_handle = arg(emu, 1);
    let name = if printer_name == 0 {
        String::new()
    } else {
        emu.memory.cstr_lossy(printer_name, 260).hle()
    };

    if out_handle == 0 {
        emu.hle.last_error = ERROR_INVALID_PARAMETER;
        ret(emu, 0);
        return HleResult::Retn(12);
    }

    let handle = emu.hle.alloc_handle(Handle::Printer { name });
    emu.memory.write_u32(out_handle, handle).hle();
    ret(emu, 1);
    HleResult::Retn(12)
}

// BOOL ClosePrinter(HANDLE hPrinter)
// Close a printer handle previously returned by OpenPrinter.
fn hle_close_printer(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let handle = arg(emu, 0);
    let Some(index) = handle.checked_sub(0x100).map(|value| value as usize) else {
        emu.hle.last_error = ERROR_INVALID_HANDLE;
        ret(emu, 0);
        return HleResult::Retn(4);
    };
    let closed = if matches!(emu.hle.handles.get(index), Some(Some(Handle::Printer { .. }))) {
        emu.hle.handles[index] = None;
        true
    } else {
        false
    };
    if !closed {
        emu.hle.last_error = ERROR_INVALID_HANDLE;
    }
    ret(emu, closed as u32);
    HleResult::Retn(4)
}

// LONG DocumentPropertiesA(HWND hWnd, HANDLE hPrinter, LPSTR pDeviceName, PDEVMODEA out, PDEVMODEA in, DWORD fMode)
// Report a minimal DEVMODEA size and fill a neutral DEVMODEA when requested.
fn hle_document_properties_a(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let printer = arg(emu, 1);
    let device_name = arg(emu, 2);
    let out = arg(emu, 3);
    let mode = arg(emu, 5);
    let printer_name = match printer.checked_sub(0x100).map(|value| value as usize) {
        Some(index) => match emu.hle.handles.get(index) {
            Some(Some(Handle::Printer { name })) => Some(name.clone()),
            _ => {
                emu.hle.last_error = ERROR_INVALID_HANDLE;
                ret(emu, u32::MAX);
                return HleResult::Retn(24);
            }
        },
        None if printer == 0 => None,
        None => {
            emu.hle.last_error = ERROR_INVALID_HANDLE;
            ret(emu, u32::MAX);
            return HleResult::Retn(24);
        }
    };

    if mode == 0 {
        ret(emu, DEVMODEA_SIZE as u32);
        return HleResult::Retn(24);
    }

    if out != 0 && (mode & DM_OUT_BUFFER) != 0 {
        write_minimal_devmode_a(emu, out, device_name, printer_name.as_deref());
    }
    ret(emu, IDOK);
    HleResult::Retn(24)
}

fn write_minimal_devmode_a(
    emu: &mut Emulator,
    out: u32,
    device_name: u32,
    printer_name: Option<&str>,
) {
    let mut devmode = [0u8; DEVMODEA_SIZE];
    let name = if device_name != 0 {
        Some(emu.memory.cstr_lossy(device_name, 32).hle())
    } else {
        printer_name.map(str::to_string)
    };
    if let Some(name) = name {
        let bytes = name.as_bytes();
        let len = bytes.len().min(31);
        devmode[..len].copy_from_slice(&bytes[..len]);
    }
    devmode[32..34].copy_from_slice(&0x0401u16.to_le_bytes());
    devmode[34..36].copy_from_slice(&0x0401u16.to_le_bytes());
    devmode[36..38].copy_from_slice(&(DEVMODEA_SIZE as u16).to_le_bytes());
    emu.memory.write_bytes(out, &devmode).hle();
}
