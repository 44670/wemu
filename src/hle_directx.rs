const DDSCAPS_PRIMARYSURFACE: u32 = 0x0000_0200;
const DDSCAPS_BACKBUFFER: u32 = 0x0000_0004;
const DDBLT_COLORFILL: u32 = 0x0000_0400;
const DDBLT_KEYSRC: u32 = 0x0000_8000;
const DDBLT_KEYSRCOVERRIDE: u32 = 0x0001_0000;
const DDBLTFAST_SRCCOLORKEY: u32 = 0x0000_0001;
const DDBLTFX_FILL_COLOR_OFFSET: u32 = 80;
const DDBLTFX_SRC_COLOR_KEY_OFFSET: u32 = 92;
const DDSD_CAPS: u32 = 0x0000_0001;
const DDSD_HEIGHT: u32 = 0x0000_0002;
const DDSD_WIDTH: u32 = 0x0000_0004;
const DDSD_PITCH: u32 = 0x0000_0008;
const DDSD_LPSURFACE: u32 = 0x0000_0800;
const DDSD_PIXELFORMAT: u32 = 0x0000_1000;
const DDPF_PALETTEINDEXED8: u32 = 0x0000_0020;
const DDPF_RGB: u32 = 0x0000_0040;
const SURFACE_GUARD_SIZE: u32 = PAGE_SIZE;



#[derive(Clone, Copy)]
struct RectI {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

impl RectI {
    fn width(self) -> i32 {
        self.right.saturating_sub(self.left)
    }

    fn height(self) -> i32 {
        self.bottom.saturating_sub(self.top)
    }
}

#[derive(Clone, Copy)]
struct SurfaceInfo {
    obj: u32,
    width: u32,
    height: u32,
    bpp: u32,
    pitch: u32,
    caps: u32,
    palette: u32,
    buffer: u32,
    color_key_low: u32,
    color_key_high: u32,
    has_color_key: bool,
}

impl SurfaceInfo {
    fn bytes_per_pixel(self) -> u32 {
        match self.bpp {
            0..=8 => 1,
            9..=16 => 2,
            17..=24 => 3,
            _ => 4,
        }
    }

    fn full_rect(self) -> RectI {
        RectI {
            left: 0,
            top: 0,
            right: self.width as i32,
            bottom: self.height as i32,
        }
    }
}



























#[cfg(test)]
mod directx_tests {
    use super::{create_surface, create_surface_with_format, fill_surface_lock_desc, read_surface_info, RectI};
    use crate::memory::PagePerm;
    use crate::Emulator;

    #[test]
    fn directdraw_surfaces_have_unmapped_guards() {
        let mut emu = Emulator::new();
        let surf = create_surface(&mut emu, 0).unwrap();
        let surface = read_surface_info(&emu, surf).unwrap();

        assert!(!emu
            .memory
            .is_mapped(surface.buffer - 1, PagePerm::READ));

        let end = surface.buffer + surface.pitch * surface.height;
        assert!(emu.memory.is_mapped(end - 1, PagePerm::READ));
        assert!(!emu.memory.is_mapped(end, PagePerm::READ));
    }

    #[test]
    fn directdraw_lock_rect_offsets_surface_pointer_but_keeps_pitch() {
        let mut emu = Emulator::new();
        emu.memory
            .map(0x0002_0000, 0x1000, PagePerm::READ | PagePerm::WRITE)
            .unwrap();

        let surf = create_surface_with_format(&mut emu, 320, 200, 16, 0).unwrap();
        let surface = read_surface_info(&emu, surf).unwrap();
        let desc = 0x0002_0000;
        let rect = RectI {
            left: 7,
            top: 11,
            right: 17,
            bottom: 21,
        };

        fill_surface_lock_desc(&mut emu, surf, desc, Some(rect)).unwrap();

        assert_eq!(emu.memory.read_u32(desc + 16).unwrap(), surface.pitch);
        assert_eq!(emu.memory.read_u32(desc + 12).unwrap(), surface.width);
        assert_eq!(
            emu.memory.read_u32(desc + 36).unwrap(),
            surface.buffer + 11 * surface.pitch + 7 * surface.bytes_per_pixel()
        );
    }
}
