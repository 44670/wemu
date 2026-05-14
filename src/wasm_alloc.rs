use std::alloc::{GlobalAlloc, Layout};
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, Ordering};

const PAGE_SIZE: usize = 65_536;
const MIN_ALIGN: usize = 16;
const HEADER_SIZE: usize = size_of::<AllocHeader>();
const MIN_FREE_SIZE: usize = size_of::<FreeBlock>();

#[global_allocator]
static ALLOCATOR: FixedWasmAllocator = FixedWasmAllocator::new();

unsafe extern "C" {
    static __heap_base: u8;
}

struct FixedWasmAllocator {
    locked: AtomicBool,
}

#[repr(C)]
struct FreeBlock {
    size: usize,
    next: *mut FreeBlock,
}

#[repr(C)]
struct AllocHeader {
    block_start: usize,
    block_size: usize,
}

static mut FREE_HEAD: *mut FreeBlock = null_mut();
static mut INITIALIZED: bool = false;

unsafe impl Sync for FixedWasmAllocator {}

impl FixedWasmAllocator {
    const fn new() -> Self {
        Self {
            locked: AtomicBool::new(false),
        }
    }

    fn lock(&self) -> AllocLock<'_> {
        while self
            .locked
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
        AllocLock { allocator: self }
    }
}

struct AllocLock<'a> {
    allocator: &'a FixedWasmAllocator,
}

impl Drop for AllocLock<'_> {
    fn drop(&mut self) {
        self.allocator.locked.store(false, Ordering::Release);
    }
}

unsafe impl GlobalAlloc for FixedWasmAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _lock = self.lock();
        unsafe {
            init_heap_once();
            alloc_locked(layout)
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
        if ptr.is_null() {
            return;
        }
        let _lock = self.lock();
        unsafe {
            let header = ptr.sub(HEADER_SIZE).cast::<AllocHeader>();
            insert_free((*header).block_start, (*header).block_size);
        }
    }
}

unsafe fn init_heap_once() {
    if INITIALIZED {
        return;
    }
    let start = align_up((&raw const __heap_base) as usize, MIN_ALIGN);
    let end = core::arch::wasm32::memory_size(0) * PAGE_SIZE;
    if end > start + MIN_FREE_SIZE {
        let block = start as *mut FreeBlock;
        (*block).size = end - start;
        (*block).next = null_mut();
        FREE_HEAD = block;
    }
    INITIALIZED = true;
}

unsafe fn alloc_locked(layout: Layout) -> *mut u8 {
    let request_size = layout.size().max(1);
    let request_align = layout.align().max(MIN_ALIGN);
    let mut prev: *mut FreeBlock = null_mut();
    let mut current = FREE_HEAD;
    while !current.is_null() {
        let block_start = current as usize;
        let block_size = (*current).size;
        let user = align_up(block_start + HEADER_SIZE, request_align);
        let alloc_end = align_up(user.saturating_add(request_size), MIN_ALIGN);
        if alloc_end >= user && alloc_end <= block_start.saturating_add(block_size) {
            let consumed = alloc_end - block_start;
            let suffix_size = block_size - consumed;
            let next = (*current).next;
            if suffix_size >= MIN_FREE_SIZE {
                let suffix = alloc_end as *mut FreeBlock;
                (*suffix).size = suffix_size;
                (*suffix).next = next;
                if prev.is_null() {
                    FREE_HEAD = suffix;
                } else {
                    (*prev).next = suffix;
                }
                write_header(user, block_start, consumed);
            } else {
                if prev.is_null() {
                    FREE_HEAD = next;
                } else {
                    (*prev).next = next;
                }
                write_header(user, block_start, block_size);
            }
            return user as *mut u8;
        }
        prev = current;
        current = (*current).next;
    }
    null_mut()
}

unsafe fn write_header(user: usize, block_start: usize, block_size: usize) {
    let header = (user - HEADER_SIZE) as *mut AllocHeader;
    (*header).block_start = block_start;
    (*header).block_size = block_size;
}

unsafe fn insert_free(block_start: usize, block_size: usize) {
    let block = block_start as *mut FreeBlock;
    (*block).size = block_size;
    (*block).next = null_mut();

    let mut prev: *mut FreeBlock = null_mut();
    let mut current = FREE_HEAD;
    while !current.is_null() && (current as usize) < block_start {
        prev = current;
        current = (*current).next;
    }

    (*block).next = current;
    if prev.is_null() {
        FREE_HEAD = block;
    } else {
        (*prev).next = block;
    }

    coalesce_after(block);
    if !prev.is_null() {
        coalesce_after(prev);
    }
}

unsafe fn coalesce_after(block: *mut FreeBlock) {
    let next = (*block).next;
    if next.is_null() {
        return;
    }
    let block_end = block as usize + (*block).size;
    if block_end == next as usize {
        (*block).size += (*next).size;
        (*block).next = (*next).next;
    }
}

fn align_up(value: usize, alignment: usize) -> usize {
    debug_assert!(alignment.is_power_of_two());
    (value + alignment - 1) & !(alignment - 1)
}
