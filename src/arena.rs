use crate::memory::{align_up, Memory, PagePerm, PAGE_SIZE};
use crate::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArenaAllocOptions {
    pub alignment: u32,
    pub guard_before: u32,
    pub guard_after: u32,
    pub protect: u32,
}

impl ArenaAllocOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn alignment(mut self, alignment: u32) -> Self {
        self.alignment = alignment;
        self
    }

    pub fn guard_before(mut self, guard_before: u32) -> Self {
        self.guard_before = guard_before;
        self
    }

    pub fn guard_after(mut self, guard_after: u32) -> Self {
        self.guard_after = guard_after;
        self
    }

    pub fn protect(mut self, protect: u32) -> Self {
        self.protect = protect;
        self
    }
}

impl Default for ArenaAllocOptions {
    fn default() -> Self {
        Self {
            alignment: PAGE_SIZE,
            guard_before: 0,
            guard_after: 0,
            protect: 0x04,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArenaAllocation {
    // base/size are the mapped payload. reserved_* also covers guard pages.
    pub base: u32,
    pub size: u32,
    pub requested_size: u32,
    pub reserved_base: u32,
    pub reserved_size: u32,
    pub guard_before: u32,
    pub guard_after: u32,
    pub protect: u32,
    pub perm: PagePerm,
}

#[derive(Clone)]
pub struct GuestArena {
    name: &'static str,
    base: u32,
    size: u32,
    allocations: Vec<ArenaAllocation>,
    retain_freed_pages: bool,
}

impl GuestArena {
    pub fn new(name: &'static str, base: u32, size: u32) -> Self {
        Self {
            name,
            base,
            size,
            allocations: Vec::new(),
            retain_freed_pages: false,
        }
    }

    pub fn retain_freed_pages(mut self) -> Self {
        self.retain_freed_pages = true;
        self
    }

    pub fn alloc(
        &mut self,
        mem: &mut Memory,
        size: u32,
        perm: PagePerm,
    ) -> Result<ArenaAllocation> {
        self.alloc_with_options(mem, size, perm, ArenaAllocOptions::default())
    }

    pub fn alloc_with_options(
        &mut self,
        mem: &mut Memory,
        size: u32,
        perm: PagePerm,
        options: ArenaAllocOptions,
    ) -> Result<ArenaAllocation> {
        let spec = AllocationSpec::new(size, options)?;
        let base = self.find_free_base(&spec)?;
        self.alloc_at_checked(mem, base, spec, perm)
    }

    pub fn alloc_at(
        &mut self,
        mem: &mut Memory,
        base: u32,
        size: u32,
        perm: PagePerm,
    ) -> Result<ArenaAllocation> {
        self.alloc_at_with_options(mem, base, size, perm, ArenaAllocOptions::default())
    }

    pub fn alloc_at_with_options(
        &mut self,
        mem: &mut Memory,
        base: u32,
        size: u32,
        perm: PagePerm,
        options: ArenaAllocOptions,
    ) -> Result<ArenaAllocation> {
        let spec = AllocationSpec::new(size, options)?;
        if base & (PAGE_SIZE - 1) != 0 {
            return Err(Error::Memory(format!(
                "{} alloc_at requires page-aligned base, got {base:08x}",
                self.name
            )));
        }
        if base as u64 % spec.alignment as u64 != 0 {
            return Err(Error::Memory(format!(
                "{} alloc_at base {base:08x} is not aligned to {:x}",
                self.name, spec.alignment
            )));
        }
        self.alloc_at_checked(mem, base, spec, perm)
    }

    pub fn free(&mut self, mem: &mut Memory, base: u32) -> Result<ArenaAllocation> {
        let index = self
            .allocations
            .iter()
            .position(|allocation| allocation.base == base)
            .ok_or_else(|| Error::Memory(format!("{} free unknown base {base:08x}", self.name)))?;
        let allocation = self.allocations.remove(index);
        if !self.retain_freed_pages {
            mem.unmap(allocation.base, allocation.size)?;
        }
        Ok(allocation)
    }

    pub fn try_free(&mut self, mem: &mut Memory, base: u32) -> Result<Option<ArenaAllocation>> {
        if self.allocation_by_base(base).is_none() {
            return Ok(None);
        }
        self.free(mem, base).map(Some)
    }

    pub fn allocation_by_base(&self, base: u32) -> Option<ArenaAllocation> {
        self.allocations
            .iter()
            .copied()
            .find(|allocation| allocation.base == base)
    }

    pub fn allocation_containing(&self, addr: u32) -> Option<ArenaAllocation> {
        self.allocations
            .iter()
            .copied()
            .find(|allocation| contains_u32(allocation.base, allocation.size, addr))
    }

    pub fn allocation_containing_range(&self, addr: u32, size: u32) -> Option<ArenaAllocation> {
        let end = (addr as u64).checked_add(size.max(1) as u64)?;
        self.allocations.iter().copied().find(|allocation| {
            let alloc_end = allocation.base as u64 + allocation.size as u64;
            addr as u64 >= allocation.base as u64 && end <= alloc_end
        })
    }

    pub fn update_metadata(
        &mut self,
        base: u32,
        protect: u32,
        perm: PagePerm,
    ) -> Result<ArenaAllocation> {
        let allocation = self
            .allocations
            .iter_mut()
            .find(|allocation| allocation.base == base)
            .ok_or_else(|| {
                Error::Memory(format!("{} update unknown base {base:08x}", self.name))
            })?;
        allocation.protect = protect;
        allocation.perm = perm;
        Ok(*allocation)
    }

    fn alloc_at_checked(
        &mut self,
        mem: &mut Memory,
        base: u32,
        spec: AllocationSpec,
        perm: PagePerm,
    ) -> Result<ArenaAllocation> {
        let allocation = self.make_allocation(base, spec, perm)?;
        if self.reserved_overlaps(allocation.reserved_base, allocation.reserved_size) {
            return Err(Error::Memory(format!(
                "{} allocation overlaps reserved span base={:08x} size={:x}",
                self.name, allocation.reserved_base, allocation.reserved_size
            )));
        }
        if self.retain_freed_pages {
            mem.map_or_update(allocation.base, allocation.size, perm)?;
        } else {
            ensure_unmapped(mem, allocation.base, allocation.size, self.name)?;
            mem.map(allocation.base, allocation.size, perm)?;
        }
        self.allocations.push(allocation);
        self.allocations
            .sort_by_key(|allocation| allocation.reserved_base);
        Ok(allocation)
    }

    fn make_allocation(
        &self,
        base: u32,
        spec: AllocationSpec,
        perm: PagePerm,
    ) -> Result<ArenaAllocation> {
        let reserved_base = base.checked_sub(spec.guard_before).ok_or_else(|| {
            Error::Memory(format!(
                "{} allocation guard underflow base={base:08x} guard={:x}",
                self.name, spec.guard_before
            ))
        })?;
        let reserved_size = spec
            .guard_before
            .checked_add(spec.payload_size)
            .and_then(|value| value.checked_add(spec.guard_after))
            .ok_or_else(|| Error::Memory(format!("{} allocation size overflow", self.name)))?;
        self.check_in_arena(reserved_base, reserved_size)?;
        Ok(ArenaAllocation {
            base,
            size: spec.payload_size,
            requested_size: spec.requested_size,
            reserved_base,
            reserved_size,
            guard_before: spec.guard_before,
            guard_after: spec.guard_after,
            protect: spec.protect,
            perm,
        })
    }

    fn find_free_base(&self, spec: &AllocationSpec) -> Result<u32> {
        // First-fit over reserved spans, not mapped spans, so guard pages keep
        // their redzone even though Memory has no backing pages for them.
        let mut hole_start = self.base as u64;
        let arena_end = self.arena_end()?;
        for allocation in &self.allocations {
            let hole_end = allocation.reserved_base as u64;
            if let Some(base) = self.fit_in_hole(hole_start, hole_end, spec)? {
                return Ok(base);
            }
            hole_start =
                hole_start.max(allocation.reserved_base as u64 + allocation.reserved_size as u64);
        }
        if let Some(base) = self.fit_in_hole(hole_start, arena_end, spec)? {
            return Ok(base);
        }
        Err(Error::Memory(format!(
            "{} out of address space for size={:x} guard_before={:x} guard_after={:x}",
            self.name, spec.payload_size, spec.guard_before, spec.guard_after
        )))
    }

    fn fit_in_hole(
        &self,
        hole_start: u64,
        hole_end: u64,
        spec: &AllocationSpec,
    ) -> Result<Option<u32>> {
        let payload_min = hole_start
            .checked_add(spec.guard_before as u64)
            .ok_or_else(|| Error::Memory(format!("{} hole overflow", self.name)))?;
        let payload_base = align_up_u64(payload_min, spec.alignment as u64)?;
        let reserved_base = payload_base
            .checked_sub(spec.guard_before as u64)
            .ok_or_else(|| Error::Memory(format!("{} guard underflow", self.name)))?;
        let reserved_end = payload_base
            .checked_add(spec.payload_size as u64)
            .and_then(|value| value.checked_add(spec.guard_after as u64))
            .ok_or_else(|| Error::Memory(format!("{} fit overflow", self.name)))?;
        if reserved_base >= hole_start && reserved_end <= hole_end {
            Ok(Some(u32::try_from(payload_base).map_err(|_| {
                Error::Memory(format!("{} fit outside 32-bit space", self.name))
            })?))
        } else {
            Ok(None)
        }
    }

    fn reserved_overlaps(&self, reserved_base: u32, reserved_size: u32) -> bool {
        let reserved_end = reserved_base as u64 + reserved_size as u64;
        self.allocations.iter().any(|allocation| {
            let other_start = allocation.reserved_base as u64;
            let other_end = other_start + allocation.reserved_size as u64;
            ranges_overlap(reserved_base as u64, reserved_end, other_start, other_end)
        })
    }

    fn check_in_arena(&self, base: u32, size: u32) -> Result<()> {
        let start = base as u64;
        let end = start
            .checked_add(size as u64)
            .ok_or_else(|| Error::Memory(format!("{} range overflow", self.name)))?;
        let arena_start = self.base as u64;
        let arena_end = self.arena_end()?;
        if start < arena_start || end > arena_end {
            return Err(Error::Memory(format!(
                "{} range outside arena base={base:08x} size={size:x} arena={:08x}..{:08x}",
                self.name, self.base, arena_end as u32
            )));
        }
        Ok(())
    }

    fn arena_end(&self) -> Result<u64> {
        (self.base as u64)
            .checked_add(self.size as u64)
            .ok_or_else(|| Error::Memory(format!("{} arena end overflow", self.name)))
    }
}

#[derive(Clone, Copy)]
struct AllocationSpec {
    requested_size: u32,
    payload_size: u32,
    guard_before: u32,
    guard_after: u32,
    alignment: u32,
    protect: u32,
}

impl AllocationSpec {
    fn new(size: u32, options: ArenaAllocOptions) -> Result<Self> {
        let requested_size = size.max(1);
        let payload_size = align_up(requested_size)?;
        let guard_before = align_up(options.guard_before)?;
        let guard_after = align_up(options.guard_after)?;
        let alignment = normalize_alignment(options.alignment)?;
        Ok(Self {
            requested_size,
            payload_size,
            guard_before,
            guard_after,
            alignment,
            protect: options.protect,
        })
    }
}

fn normalize_alignment(alignment: u32) -> Result<u32> {
    let alignment = alignment.max(PAGE_SIZE);
    if !alignment.is_power_of_two() {
        return Err(Error::Memory(format!(
            "arena allocation alignment must be a power of two, got {alignment:x}"
        )));
    }
    Ok(alignment)
}

fn align_up_u64(value: u64, alignment: u64) -> Result<u64> {
    value
        .checked_add(alignment - 1)
        .map(|value| value & !(alignment - 1))
        .ok_or_else(|| Error::Memory(format!("arena align overflow: {value:x}")))
}

fn contains_u32(base: u32, size: u32, addr: u32) -> bool {
    let start = base as u64;
    let end = start + size as u64;
    let addr = addr as u64;
    addr >= start && addr < end
}

fn ranges_overlap(lhs_start: u64, lhs_end: u64, rhs_start: u64, rhs_end: u64) -> bool {
    lhs_start < rhs_end && rhs_start < lhs_end
}

fn ensure_unmapped(mem: &Memory, base: u32, size: u32, arena: &str) -> Result<()> {
    let end = (base as u64)
        .checked_add(size as u64)
        .ok_or_else(|| Error::Memory(format!("{arena} mapped-page check overflow")))?;
    let mut addr = base as u64;
    while addr < end {
        let page = u32::try_from(addr)
            .map_err(|_| Error::Memory(format!("{arena} page address outside 32-bit space")))?;
        if mem.is_mapped(page, PagePerm::READ) {
            return Err(Error::Memory(format!(
                "{arena} allocation payload overlaps mapped page {page:08x}"
            )));
        }
        addr += PAGE_SIZE as u64;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{ArenaAllocOptions, GuestArena};
    use crate::memory::{Memory, PagePerm};

    #[test]
    fn alloc_reuses_first_fit_hole_after_free() {
        let mut memory = Memory::new();
        let mut arena = GuestArena::new("test", 0x0010_0000, 0x0010_0000);

        let first = arena
            .alloc(&mut memory, 0x2000, PagePerm::READ | PagePerm::WRITE)
            .unwrap();
        let second = arena
            .alloc(&mut memory, 0x3000, PagePerm::READ | PagePerm::WRITE)
            .unwrap();
        arena.free(&mut memory, first.base).unwrap();
        let reused = arena
            .alloc(&mut memory, 0x1000, PagePerm::READ | PagePerm::WRITE)
            .unwrap();

        assert_eq!(reused.base, first.base);
        assert_eq!(second.base, 0x0010_2000);
        assert!(memory.is_mapped(reused.base, PagePerm::WRITE));
        assert!(!memory.is_mapped(first.base + 0x1000, PagePerm::WRITE));
    }

    #[test]
    fn alloc_at_rejects_collision_and_out_of_range() {
        let mut memory = Memory::new();
        let mut arena = GuestArena::new("test", 0x0100_0000, 0x0002_0000);

        arena
            .alloc_at(
                &mut memory,
                0x0100_8000,
                0x4000,
                PagePerm::READ | PagePerm::WRITE,
            )
            .unwrap();

        assert!(arena
            .alloc_at(
                &mut memory,
                0x0100_a000,
                0x1000,
                PagePerm::READ | PagePerm::WRITE,
            )
            .is_err());
        assert!(arena
            .alloc_at(
                &mut memory,
                0x0102_0000,
                0x1000,
                PagePerm::READ | PagePerm::WRITE,
            )
            .is_err());
    }

    #[test]
    fn guard_pages_are_reserved_but_unmapped() {
        let mut memory = Memory::new();
        let mut arena = GuestArena::new("test", 0x0200_0000, 0x0002_0000);

        let guarded = arena
            .alloc_with_options(
                &mut memory,
                0x1000,
                PagePerm::READ | PagePerm::WRITE,
                ArenaAllocOptions::new().guard_after(0x2000),
            )
            .unwrap();
        let next = arena
            .alloc(&mut memory, 0x1000, PagePerm::READ | PagePerm::WRITE)
            .unwrap();

        assert!(memory.is_mapped(guarded.base, PagePerm::WRITE));
        assert!(!memory.is_mapped(guarded.base + 0x1000, PagePerm::WRITE));
        assert!(!memory.is_mapped(guarded.base + 0x2000, PagePerm::WRITE));
        assert_eq!(next.base, guarded.base + 0x3000);
    }

    #[test]
    fn alloc_alignment_is_honored() {
        let mut memory = Memory::new();
        let mut arena = GuestArena::new("test", 0x0300_1000, 0x0004_0000);

        let allocation = arena
            .alloc_with_options(
                &mut memory,
                0x1000,
                PagePerm::READ | PagePerm::WRITE,
                ArenaAllocOptions::new().alignment(0x1_0000),
            )
            .unwrap();

        assert_eq!(allocation.base, 0x0301_0000);
    }
}
