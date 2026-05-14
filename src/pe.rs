use std::fs;
use std::path::{Path, PathBuf};

use crate::hle::Hle;
use crate::memory::{align_up, Memory, PagePerm};
use crate::{Error, Result};

const IMAGE_DOS_SIGNATURE: u16 = 0x5a4d;
const IMAGE_NT_SIGNATURE: u32 = 0x0000_4550;
const IMAGE_FILE_MACHINE_I386: u16 = 0x014c;
const IMAGE_NT_OPTIONAL_HDR32_MAGIC: u16 = 0x10b;
const IMAGE_DIRECTORY_ENTRY_EXPORT: usize = 0;
const IMAGE_DIRECTORY_ENTRY_IMPORT: usize = 1;
const IMAGE_DIRECTORY_ENTRY_BASERELOC: usize = 5;
const IMAGE_REL_BASED_ABSOLUTE: u16 = 0;
const IMAGE_REL_BASED_HIGHLOW: u16 = 3;

#[derive(Debug, Clone)]
pub struct PeImage {
    pub path: PathBuf,
    pub image_base: u32,
    pub entry: u32,
    pub size_of_image: u32,
    pub sections: Vec<Section>,
    pub imports: Vec<Import>,
    pub exports: Vec<Export>,
}

#[derive(Debug, Clone)]
pub struct Section {
    pub name: String,
    pub va: u32,
    pub virtual_size: u32,
    pub raw_size: u32,
    pub characteristics: u32,
}

#[derive(Debug, Clone)]
pub struct Import {
    pub dll: String,
    pub name: String,
    pub iat_va: u32,
    pub target: u32,
}

#[derive(Debug, Clone)]
pub struct Export {
    pub name: Option<String>,
    pub ordinal: u32,
    pub addr: u32,
}

impl PeImage {
    pub fn resolve_export(&self, name: &str) -> Option<u32> {
        if let Some(ordinal) = name.strip_prefix('#').and_then(|n| n.parse::<u32>().ok()) {
            return self
                .exports
                .iter()
                .find(|export| export.ordinal == ordinal)
                .map(|export| export.addr);
        }
        self.exports
            .iter()
            .find(|export| export.name.as_deref() == Some(name))
            .map(|export| export.addr)
    }
}

pub fn load_pe32(path: &Path, mem: &mut Memory, hle: &mut Hle) -> Result<PeImage> {
    let file = fs::read(path)?;
    load_pe32_bytes(path.to_path_buf(), &file, mem, hle)
}

pub fn load_pe32_dll(path: &Path, mem: &mut Memory, hle: &mut Hle) -> Result<PeImage> {
    let file = fs::read(path)?;
    load_pe32_relocatable_bytes(path.to_path_buf(), &file, mem, hle)
}

pub fn load_pe32_dll_bytes(
    path: PathBuf,
    file: &[u8],
    mem: &mut Memory,
    hle: &mut Hle,
) -> Result<PeImage> {
    load_pe32_relocatable_bytes(path, file, mem, hle)
}

pub fn load_pe32_resource_image(path: &Path, mem: &mut Memory, hle: &mut Hle) -> Result<PeImage> {
    let file = fs::read(path)?;
    load_pe32_resource_image_bytes(path.to_path_buf(), &file, mem, hle)
}

fn load_pe32_resource_image_bytes(
    path: PathBuf,
    file: &[u8],
    mem: &mut Memory,
    hle: &mut Hle,
) -> Result<PeImage> {
    if file.len() < 0x100 {
        return Err(Error::Pe("file too small".to_string()));
    }
    if read_u16(file, 0)? != IMAGE_DOS_SIGNATURE {
        return Err(Error::Pe("missing MZ signature".to_string()));
    }
    let pe_off = read_u32(file, 0x3c)? as usize;
    if read_u32(file, pe_off)? != IMAGE_NT_SIGNATURE {
        return Err(Error::Pe("missing PE signature".to_string()));
    }

    let coff = pe_off + 4;
    let machine = read_u16(file, coff)?;
    if machine != IMAGE_FILE_MACHINE_I386 {
        return Err(Error::Pe(format!("unsupported machine {machine:04x}")));
    }
    let section_count = read_u16(file, coff + 2)? as usize;
    let optional_size = read_u16(file, coff + 16)? as usize;
    let optional = coff + 20;
    if read_u16(file, optional)? != IMAGE_NT_OPTIONAL_HDR32_MAGIC {
        return Err(Error::Pe("not a PE32 image".to_string()));
    }

    let address_of_entry_point = read_u32(file, optional + 0x10)?;
    let size_of_image = read_u32(file, optional + 0x38)?;
    let size_of_headers = read_u32(file, optional + 0x3c)?;
    let image_base = hle.alloc_module_image(mem, size_of_image)?;

    let header_copy = (size_of_headers as usize).min(file.len());
    mem.write_bytes(image_base, &file[..header_copy])?;

    let section_table = optional + optional_size;
    let mut sections = Vec::with_capacity(section_count);
    for i in 0..section_count {
        let off = section_table + i * 40;
        let name = section_name(&file[off..off + 8]);
        let virtual_size = read_u32(file, off + 8)?;
        let va = read_u32(file, off + 12)?;
        let raw_size = read_u32(file, off + 16)?;
        let raw_ptr = read_u32(file, off + 20)?;
        let characteristics = read_u32(file, off + 36)?;
        if raw_size != 0 && raw_ptr != 0 {
            let begin = raw_ptr as usize;
            let end = begin.saturating_add(raw_size as usize).min(file.len());
            if begin > file.len() {
                return Err(Error::Pe(format!(
                    "section {name} raw pointer outside file"
                )));
            }
            mem.write_bytes(image_base.wrapping_add(va), &file[begin..end])?;
        }
        sections.push(Section {
            name,
            va: image_base.wrapping_add(va),
            virtual_size,
            raw_size,
            characteristics,
        });
    }

    Ok(PeImage {
        path,
        image_base,
        entry: image_base.wrapping_add(address_of_entry_point),
        size_of_image,
        sections,
        imports: Vec::new(),
        exports: Vec::new(),
    })
}

fn load_pe32_relocatable_bytes(
    path: PathBuf,
    file: &[u8],
    mem: &mut Memory,
    hle: &mut Hle,
) -> Result<PeImage> {
    if file.len() < 0x100 {
        return Err(Error::Pe("file too small".to_string()));
    }
    if read_u16(file, 0)? != IMAGE_DOS_SIGNATURE {
        return Err(Error::Pe("missing MZ signature".to_string()));
    }
    let pe_off = read_u32(file, 0x3c)? as usize;
    if read_u32(file, pe_off)? != IMAGE_NT_SIGNATURE {
        return Err(Error::Pe("missing PE signature".to_string()));
    }

    let coff = pe_off + 4;
    let machine = read_u16(file, coff)?;
    if machine != IMAGE_FILE_MACHINE_I386 {
        return Err(Error::Pe(format!("unsupported machine {machine:04x}")));
    }
    let section_count = read_u16(file, coff + 2)? as usize;
    let optional_size = read_u16(file, coff + 16)? as usize;
    let optional = coff + 20;
    if read_u16(file, optional)? != IMAGE_NT_OPTIONAL_HDR32_MAGIC {
        return Err(Error::Pe("not a PE32 image".to_string()));
    }

    let address_of_entry_point = read_u32(file, optional + 0x10)?;
    let preferred_base = read_u32(file, optional + 0x1c)?;
    let section_alignment = read_u32(file, optional + 0x20)?;
    let size_of_image = read_u32(file, optional + 0x38)?;
    let size_of_headers = read_u32(file, optional + 0x3c)?;
    let number_of_rva_and_sizes = read_u32(file, optional + 0x5c)? as usize;
    let data_dir = optional + 0x60;

    if section_alignment == 0 {
        return Err(Error::Pe("section alignment is zero".to_string()));
    }
    let image_base = hle.alloc_module_image(mem, size_of_image)?;

    let header_map_size = align_up(size_of_headers)?;
    mem.map_or_update(
        image_base,
        header_map_size,
        PagePerm::READ | PagePerm::WRITE,
    )?;
    let header_copy = (size_of_headers as usize).min(file.len());
    mem.write_bytes(image_base, &file[..header_copy])?;

    let section_table = optional + optional_size;
    let mut sections = Vec::with_capacity(section_count);
    let mut pending_protect = Vec::with_capacity(section_count);
    for i in 0..section_count {
        let off = section_table + i * 40;
        let name = section_name(&file[off..off + 8]);
        let virtual_size = read_u32(file, off + 8)?;
        let va = read_u32(file, off + 12)?;
        let raw_size = read_u32(file, off + 16)?;
        let raw_ptr = read_u32(file, off + 20)?;
        let characteristics = read_u32(file, off + 36)?;
        let map_size = align_up(virtual_size.max(raw_size).max(1))?;
        let dst = image_base.wrapping_add(va);
        mem.map_or_update(
            dst,
            map_size,
            PagePerm::READ | PagePerm::WRITE | PagePerm::EXEC,
        )?;
        if raw_size != 0 && raw_ptr != 0 {
            let begin = raw_ptr as usize;
            let end = begin.saturating_add(raw_size as usize).min(file.len());
            if begin > file.len() {
                return Err(Error::Pe(format!(
                    "section {name} raw pointer outside file"
                )));
            }
            mem.write_bytes(dst, &file[begin..end])?;
        }
        pending_protect.push((dst, map_size, section_perm(characteristics)));
        sections.push(Section {
            name,
            va: dst,
            virtual_size,
            raw_size,
            characteristics,
        });
    }

    let mut imports = Vec::new();
    if number_of_rva_and_sizes > IMAGE_DIRECTORY_ENTRY_IMPORT {
        let import_rva = read_u32(file, data_dir + IMAGE_DIRECTORY_ENTRY_IMPORT * 8)?;
        let import_size = read_u32(file, data_dir + IMAGE_DIRECTORY_ENTRY_IMPORT * 8 + 4)?;
        if import_rva != 0 && import_size != 0 {
            imports = bind_imports(image_base, import_rva, mem, hle)?;
        }
    }

    if image_base != preferred_base {
        if number_of_rva_and_sizes <= IMAGE_DIRECTORY_ENTRY_BASERELOC {
            return Err(Error::Pe(format!(
                "image loaded at {image_base:08x} but has no relocation directory"
            )));
        }
        let reloc_rva = read_u32(file, data_dir + IMAGE_DIRECTORY_ENTRY_BASERELOC * 8)?;
        let reloc_size = read_u32(file, data_dir + IMAGE_DIRECTORY_ENTRY_BASERELOC * 8 + 4)?;
        if reloc_rva == 0 || reloc_size == 0 {
            return Err(Error::Pe(format!(
                "image loaded at {image_base:08x} but relocations are stripped"
            )));
        }
        apply_base_relocations(image_base, preferred_base, reloc_rva, reloc_size, mem)?;
    }

    let exports = if number_of_rva_and_sizes > IMAGE_DIRECTORY_ENTRY_EXPORT {
        let export_rva = read_u32(file, data_dir + IMAGE_DIRECTORY_ENTRY_EXPORT * 8)?;
        let export_size = read_u32(file, data_dir + IMAGE_DIRECTORY_ENTRY_EXPORT * 8 + 4)?;
        parse_exports(image_base, export_rva, export_size, mem)?
    } else {
        Vec::new()
    };

    for (addr, size, perm) in pending_protect {
        mem.protect(addr, size, perm)?;
    }

    Ok(PeImage {
        path,
        image_base,
        entry: image_base.wrapping_add(address_of_entry_point),
        size_of_image,
        sections,
        imports,
        exports,
    })
}

pub fn load_pe32_bytes(
    path: PathBuf,
    file: &[u8],
    mem: &mut Memory,
    hle: &mut Hle,
) -> Result<PeImage> {
    if file.len() < 0x100 {
        return Err(Error::Pe("file too small".to_string()));
    }
    if read_u16(&file, 0)? != IMAGE_DOS_SIGNATURE {
        return Err(Error::Pe("missing MZ signature".to_string()));
    }
    let pe_off = read_u32(&file, 0x3c)? as usize;
    if read_u32(&file, pe_off)? != IMAGE_NT_SIGNATURE {
        return Err(Error::Pe("missing PE signature".to_string()));
    }

    let coff = pe_off + 4;
    let machine = read_u16(&file, coff)?;
    if machine != IMAGE_FILE_MACHINE_I386 {
        return Err(Error::Pe(format!("unsupported machine {machine:04x}")));
    }
    let section_count = read_u16(&file, coff + 2)? as usize;
    let optional_size = read_u16(&file, coff + 16)? as usize;
    let optional = coff + 20;
    if read_u16(&file, optional)? != IMAGE_NT_OPTIONAL_HDR32_MAGIC {
        return Err(Error::Pe("not a PE32 image".to_string()));
    }

    let address_of_entry_point = read_u32(&file, optional + 0x10)?;
    let image_base = read_u32(&file, optional + 0x1c)?;
    let section_alignment = read_u32(&file, optional + 0x20)?;
    let size_of_image = read_u32(&file, optional + 0x38)?;
    let size_of_headers = read_u32(&file, optional + 0x3c)?;
    let number_of_rva_and_sizes = read_u32(&file, optional + 0x5c)? as usize;
    let data_dir = optional + 0x60;

    if section_alignment == 0 {
        return Err(Error::Pe("section alignment is zero".to_string()));
    }
    hle.reserve_module_image(mem, image_base, size_of_image)
        .map_err(|err| {
            Error::Pe(format!(
                "cannot reserve PE image at {image_base:08x}: {err}"
            ))
        })?;

    let header_map_size = align_up(size_of_headers)?;
    mem.map_or_update(
        image_base,
        header_map_size,
        PagePerm::READ | PagePerm::WRITE,
    )?;
    let header_copy = (size_of_headers as usize).min(file.len());
    mem.write_bytes(image_base, &file[..header_copy])?;

    let section_table = optional + optional_size;
    let mut sections = Vec::with_capacity(section_count);
    let mut pending_protect = Vec::with_capacity(section_count);
    for i in 0..section_count {
        let off = section_table + i * 40;
        let name = section_name(&file[off..off + 8]);
        let virtual_size = read_u32(&file, off + 8)?;
        let va = read_u32(&file, off + 12)?;
        let raw_size = read_u32(&file, off + 16)?;
        let raw_ptr = read_u32(&file, off + 20)?;
        let characteristics = read_u32(&file, off + 36)?;
        let map_size = align_up(virtual_size.max(raw_size).max(1))?;
        let dst = image_base.wrapping_add(va);
        mem.map_or_update(
            dst,
            map_size,
            PagePerm::READ | PagePerm::WRITE | PagePerm::EXEC,
        )?;
        if raw_size != 0 && raw_ptr != 0 {
            let begin = raw_ptr as usize;
            let end = begin.saturating_add(raw_size as usize).min(file.len());
            if begin > file.len() {
                return Err(Error::Pe(format!(
                    "section {name} raw pointer outside file"
                )));
            }
            mem.write_bytes(dst, &file[begin..end])?;
        }
        pending_protect.push((dst, map_size, section_perm(characteristics)));
        sections.push(Section {
            name,
            va: dst,
            virtual_size,
            raw_size,
            characteristics,
        });
    }

    let mut imports = Vec::new();
    if number_of_rva_and_sizes > IMAGE_DIRECTORY_ENTRY_IMPORT {
        let import_rva = read_u32(&file, data_dir + IMAGE_DIRECTORY_ENTRY_IMPORT * 8)?;
        let import_size = read_u32(&file, data_dir + IMAGE_DIRECTORY_ENTRY_IMPORT * 8 + 4)?;
        if import_rva != 0 && import_size != 0 {
            imports = bind_imports(image_base, import_rva, mem, hle)?;
        }
    }

    let exports = if number_of_rva_and_sizes > IMAGE_DIRECTORY_ENTRY_EXPORT {
        let export_rva = read_u32(&file, data_dir + IMAGE_DIRECTORY_ENTRY_EXPORT * 8)?;
        let export_size = read_u32(&file, data_dir + IMAGE_DIRECTORY_ENTRY_EXPORT * 8 + 4)?;
        parse_exports(image_base, export_rva, export_size, mem)?
    } else {
        Vec::new()
    };

    for (addr, size, perm) in pending_protect {
        mem.protect(addr, size, perm)?;
    }

    Ok(PeImage {
        path,
        image_base,
        entry: image_base.wrapping_add(address_of_entry_point),
        size_of_image,
        sections,
        imports,
        exports,
    })
}

fn bind_imports(
    image_base: u32,
    import_rva: u32,
    mem: &mut Memory,
    hle: &mut Hle,
) -> Result<Vec<Import>> {
    let mut imports = Vec::new();
    let mut desc = image_base.wrapping_add(import_rva);
    loop {
        let original_first_thunk = mem.read_u32(desc)?;
        let name_rva = mem.read_u32(desc + 12)?;
        let first_thunk = mem.read_u32(desc + 16)?;
        if original_first_thunk == 0 && name_rva == 0 && first_thunk == 0 {
            break;
        }
        let dll = mem.cstr_lossy(image_base.wrapping_add(name_rva), 260)?;
        let lookup_rva = if original_first_thunk != 0 {
            original_first_thunk
        } else {
            first_thunk
        };
        let mut lookup = image_base.wrapping_add(lookup_rva);
        let mut iat = image_base.wrapping_add(first_thunk);
        loop {
            let thunk = mem.read_u32(lookup)?;
            if thunk == 0 {
                break;
            }
            let name = if (thunk & 0x8000_0000) != 0 {
                format!("#{}", thunk & 0xffff)
            } else {
                mem.cstr_lossy(image_base.wrapping_add(thunk).wrapping_add(2), 256)?
            };
            let target = hle.resolve_pe_import(mem, &dll, &name)?;
            mem.write_u32(iat, target)?;
            imports.push(Import {
                dll: dll.clone(),
                name,
                iat_va: iat,
                target,
            });
            lookup = lookup.wrapping_add(4);
            iat = iat.wrapping_add(4);
        }
        desc = desc.wrapping_add(20);
    }
    Ok(imports)
}

fn apply_base_relocations(
    image_base: u32,
    preferred_base: u32,
    reloc_rva: u32,
    reloc_size: u32,
    mem: &mut Memory,
) -> Result<()> {
    let delta = image_base.wrapping_sub(preferred_base);
    if delta == 0 {
        return Ok(());
    }

    let mut block = image_base.wrapping_add(reloc_rva);
    let end = block.wrapping_add(reloc_size);
    while block.wrapping_add(8) <= end {
        let page_rva = mem.read_u32(block)?;
        let block_size = mem.read_u32(block + 4)?;
        if block_size == 0 {
            break;
        }
        if block_size < 8 || block.wrapping_add(block_size) > end {
            return Err(Error::Pe(format!(
                "invalid relocation block size {block_size:x} at {block:08x}"
            )));
        }

        let entries = (block_size - 8) / 2;
        for i in 0..entries {
            let entry = mem.read_u16(block + 8 + i * 2)?;
            let reloc_type = entry >> 12;
            let offset = (entry & 0x0fff) as u32;
            match reloc_type {
                IMAGE_REL_BASED_ABSOLUTE => {}
                IMAGE_REL_BASED_HIGHLOW => {
                    let patch_addr = image_base.wrapping_add(page_rva).wrapping_add(offset);
                    let old = mem.read_u32(patch_addr)?;
                    mem.write_u32(patch_addr, old.wrapping_add(delta))?;
                }
                _ => {
                    return Err(Error::Pe(format!(
                        "unsupported relocation type {reloc_type} at {block:08x}"
                    )));
                }
            }
        }
        block = block.wrapping_add(block_size);
    }
    Ok(())
}

fn parse_exports(
    image_base: u32,
    export_rva: u32,
    export_size: u32,
    mem: &Memory,
) -> Result<Vec<Export>> {
    if export_rva == 0 || export_size == 0 {
        return Ok(Vec::new());
    }

    let dir = image_base.wrapping_add(export_rva);
    let ordinal_base = mem.read_u32(dir + 16)?;
    let address_count = mem.read_u32(dir + 20)? as usize;
    let name_count = mem.read_u32(dir + 24)? as usize;
    let functions = image_base.wrapping_add(mem.read_u32(dir + 28)?);
    let names = image_base.wrapping_add(mem.read_u32(dir + 32)?);
    let ordinals = image_base.wrapping_add(mem.read_u32(dir + 36)?);

    let mut names_by_index = vec![None; address_count];
    for i in 0..name_count {
        let name_rva = mem.read_u32(names.wrapping_add((i * 4) as u32))?;
        let ordinal_index = mem.read_u16(ordinals.wrapping_add((i * 2) as u32))? as usize;
        if ordinal_index >= names_by_index.len() {
            continue;
        }
        let name = mem.cstr_lossy(image_base.wrapping_add(name_rva), 256)?;
        names_by_index[ordinal_index] = Some(name);
    }

    let forwarder_end = export_rva.saturating_add(export_size);
    let mut exports = Vec::new();
    for (index, name) in names_by_index.into_iter().enumerate() {
        let rva = mem.read_u32(functions.wrapping_add((index * 4) as u32))?;
        if rva == 0 || (rva >= export_rva && rva < forwarder_end) {
            continue;
        }
        exports.push(Export {
            name,
            ordinal: ordinal_base.wrapping_add(index as u32),
            addr: image_base.wrapping_add(rva),
        });
    }
    Ok(exports)
}

fn section_perm(characteristics: u32) -> PagePerm {
    let mut perm = PagePerm::READ;
    if (characteristics & 0x8000_0000) != 0 {
        perm = perm | PagePerm::WRITE;
    }
    if (characteristics & 0x2000_0000) != 0 {
        perm = perm | PagePerm::EXEC;
    }
    perm
}

fn section_name(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

fn read_u16(file: &[u8], off: usize) -> Result<u16> {
    let bytes = file
        .get(off..off + 2)
        .ok_or_else(|| Error::Pe(format!("read_u16 outside file at {off:x}")))?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u32(file: &[u8], off: usize) -> Result<u32> {
    let bytes = file
        .get(off..off + 4)
        .ok_or_else(|| Error::Pe(format!("read_u32 outside file at {off:x}")))?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_name_stops_at_nul() {
        assert_eq!(section_name(b".text\0\0\0"), ".text");
    }

    #[test]
    fn resolve_export_handles_names_and_ordinals() {
        let image = PeImage {
            path: std::path::PathBuf::from("test.dll"),
            image_base: 0x0010_0000,
            entry: 0,
            size_of_image: 0x1000,
            sections: Vec::new(),
            imports: Vec::new(),
            exports: vec![
                Export {
                    name: Some("NamedProc".to_string()),
                    ordinal: 3,
                    addr: 0x0010_1234,
                },
                Export {
                    name: None,
                    ordinal: 7,
                    addr: 0x0010_5678,
                },
            ],
        };

        assert_eq!(image.resolve_export("NamedProc"), Some(0x0010_1234));
        assert_eq!(image.resolve_export("#3"), Some(0x0010_1234));
        assert_eq!(image.resolve_export("#7"), Some(0x0010_5678));
        assert_eq!(image.resolve_export("MissingProc"), None);
    }
}
