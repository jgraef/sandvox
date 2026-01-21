use std::{
    alloc::{
        AllocError,
        Allocator,
        GlobalAlloc,
        Layout,
        System,
    },
    cmp,
    ptr::NonNull,
    sync::atomic::{
        self,
        AtomicUsize,
    },
};

#[global_allocator]
static GLOBAL: StatsAllocator<System> = StatsAllocator::new(System);

pub fn bytes_allocated() -> usize {
    GLOBAL.bytes_allocated()
}

#[derive(Debug)]
struct StatsAllocator<A> {
    inner: A,
    bytes_allocated: AtomicUsize,
}

impl<A> StatsAllocator<A> {
    #[inline]
    pub const fn new(inner: A) -> Self {
        Self {
            inner,
            bytes_allocated: AtomicUsize::new(0),
        }
    }

    #[inline]
    pub fn bytes_allocated(&self) -> usize {
        self.bytes_allocated.load(atomic::Ordering::Relaxed)
    }
}

impl<A> StatsAllocator<A> {
    #[inline]
    fn increment_bytes_allocated(&self, size: usize) {
        self.bytes_allocated
            .fetch_add(size, atomic::Ordering::Relaxed);
    }

    #[inline]
    fn decrement_bytes_allocated(&self, size: usize) {
        self.bytes_allocated
            .fetch_sub(size, atomic::Ordering::Relaxed);
    }
}

unsafe impl<A: Allocator> Allocator for StatsAllocator<A> {
    #[inline]
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        self.increment_bytes_allocated(layout.size());
        self.inner.allocate(layout)
    }

    #[inline]
    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        self.decrement_bytes_allocated(layout.size());
        unsafe { self.inner.deallocate(ptr, layout) }
    }

    #[inline]
    fn allocate_zeroed(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        self.increment_bytes_allocated(layout.size());
        self.inner.allocate_zeroed(layout)
    }

    #[inline]
    unsafe fn grow(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> Result<NonNull<[u8]>, AllocError> {
        self.increment_bytes_allocated(new_layout.size() - old_layout.size());
        unsafe { self.inner.grow(ptr, old_layout, new_layout) }
    }

    #[inline]
    unsafe fn grow_zeroed(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> Result<NonNull<[u8]>, AllocError> {
        self.increment_bytes_allocated(new_layout.size() - old_layout.size());
        unsafe { self.inner.grow_zeroed(ptr, old_layout, new_layout) }
    }

    #[inline]
    unsafe fn shrink(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> Result<NonNull<[u8]>, AllocError> {
        self.decrement_bytes_allocated(old_layout.size() - new_layout.size());
        unsafe { self.inner.shrink(ptr, old_layout, new_layout) }
    }

    #[inline]
    fn by_ref(&self) -> &Self
    where
        Self: Sized,
    {
        self
    }
}

unsafe impl<A: GlobalAlloc> GlobalAlloc for StatsAllocator<A> {
    #[inline]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.increment_bytes_allocated(layout.size());
        unsafe { self.inner.alloc(layout) }
    }

    #[inline]
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        self.increment_bytes_allocated(layout.size());
        unsafe { self.inner.alloc_zeroed(layout) }
    }

    #[inline]
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        self.decrement_bytes_allocated(layout.size());
        unsafe {
            self.inner.dealloc(ptr, layout);
        }
    }

    #[inline]
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let old_size = layout.size();
        match new_size.cmp(&old_size) {
            cmp::Ordering::Less => self.decrement_bytes_allocated(old_size - new_size),
            cmp::Ordering::Greater => self.increment_bytes_allocated(new_size - old_size),
            _ => {}
        }
        unsafe { self.inner.realloc(ptr, layout, new_size) }
    }
}
