//! Safe wrappers over the HarfBuzz subsetter calls craft-font-subset makes.

#![warn(missing_docs)]

use alloc::vec::Vec;
use core::marker::PhantomData;
use core::ptr::{self, NonNull};

use crate as sys;

/// A HarfBuzz face over borrowed font bytes.
pub struct Face<'a> {
    raw: NonNull<sys::hb_face_t>,
    _bytes: PhantomData<&'a [u8]>,
}

impl<'a> Face<'a> {
    /// Face `index` of `bytes` (0 for a single font; a face of a collection otherwise). `None` when HarfBuzz
    /// cannot take the bytes; a face it cannot read subsets to nothing.
    pub fn new(bytes: &'a [u8], index: u32) -> Option<Self> {
        let len = u32::try_from(bytes.len()).ok()?;
        // SAFETY: `bytes` is valid for `len` bytes. READONLY mode makes
        // HarfBuzz read them in place and never write them; with no destroy
        // callback nothing runs when the blob is released.
        let blob =
            NonNull::new(unsafe { sys::hb_blob_create_or_fail(bytes.as_ptr().cast(), len, sys::HB_MEMORY_MODE_READONLY, ptr::null_mut(), None) })?;
        // SAFETY: `blob` is live. The face takes its own reference to it, so
        // the blob can be released right after; the face, and every blob
        // HarfBuzz derives from it, points into `bytes`, which `'a` keeps
        // borrowed for the life of `Face`.
        let face = unsafe {
            let face = sys::hb_face_create(blob.as_ptr(), index);
            sys::hb_blob_destroy(blob.as_ptr());
            face
        };
        Some(Self { raw: NonNull::new(face)?, _bytes: PhantomData })
    }

    /// Subsets the face as `input` describes and serialises the result.
    pub fn subset(&self, input: &Input) -> Option<Vec<u8>> {
        // SAFETY: both objects are live; HarfBuzz only reads them.
        let out = unsafe { sys::hb_subset_or_fail(self.raw.as_ptr(), input.raw.as_ptr()) };
        // The subset face may share table blobs with `self`; it is released
        // before this function returns.
        let out = Owned(NonNull::new(out)?, sys::hb_face_destroy);
        // SAFETY: `out` is live. A subset face serialises into a new blob
        // that owns its data.
        let blob = Owned(NonNull::new(unsafe { sys::hb_face_reference_blob(out.0.as_ptr()) })?, sys::hb_blob_destroy);
        let mut len = 0;
        // SAFETY: `blob` is live and `len` is a valid out pointer.
        let data = unsafe { sys::hb_blob_get_data(blob.0.as_ptr(), &mut len) };
        // An empty blob means HarfBuzz failed to serialise the subset.
        if data.is_null() || len == 0 {
            return None;
        }
        // SAFETY: HarfBuzz reports `len` readable bytes at `data`, which stay
        // valid until `blob` drops after the copy.
        Some(unsafe { core::slice::from_raw_parts(data.cast::<u8>(), len as usize) }.to_vec())
    }
}

impl Drop for Face<'_> {
    fn drop(&mut self) {
        // SAFETY: `raw` holds the reference `hb_face_create` returned.
        unsafe { sys::hb_face_destroy(self.raw.as_ptr()) }
    }
}

/// A HarfBuzz object reference released with `.1` on drop.
struct Owned<T>(NonNull<T>, unsafe extern "C" fn(*mut T));

impl<T> Drop for Owned<T> {
    fn drop(&mut self) {
        // SAFETY: `.0` holds a reference the caller owns, and `.1` is the
        // matching destroy function.
        unsafe { (self.1)(self.0.as_ptr()) }
    }
}

/// Subset options: HarfBuzz's defaults until changed.
pub struct Input {
    raw: NonNull<sys::hb_subset_input_t>,
}

impl Input {
    /// New options; `None` when HarfBuzz cannot allocate them.
    pub fn new() -> Option<Self> {
        // SAFETY: no preconditions; null means allocation failed.
        let raw = NonNull::new(unsafe { sys::hb_subset_input_create_or_fail() })?;
        Some(Self { raw })
    }

    /// Sets `flags` on top of the current ones.
    pub fn add_flags(&mut self, flags: u32) {
        // SAFETY: `raw` is live.
        unsafe {
            let current = sys::hb_subset_input_get_flags(self.raw.as_ptr());
            sys::hb_subset_input_set_flags(self.raw.as_ptr(), current | flags);
        }
    }

    /// The codepoints to keep.
    pub fn unicodes(&mut self) -> Set<'_> {
        // SAFETY: `raw` is live; the set belongs to the input, which the
        // returned borrow keeps alive.
        Set::new(unsafe { sys::hb_subset_input_unicode_set(self.raw.as_ptr()) })
    }

    /// The name IDs to keep (0-6 by default).
    pub fn name_ids(&mut self) -> Set<'_> {
        // SAFETY: as for `unicodes`.
        Set::new(unsafe { sys::hb_subset_input_set(self.raw.as_ptr(), sys::HB_SUBSET_SETS_NAME_ID) })
    }

    /// The table tags HarfBuzz drops outright.
    pub fn drop_tables(&mut self) -> Set<'_> {
        // SAFETY: as for `unicodes`.
        Set::new(unsafe { sys::hb_subset_input_set(self.raw.as_ptr(), sys::HB_SUBSET_SETS_DROP_TABLE_TAG) })
    }

    /// The table tags HarfBuzz copies unchanged instead of subsetting.
    pub fn no_subset_tables(&mut self) -> Set<'_> {
        // SAFETY: as for `unicodes`.
        Set::new(unsafe { sys::hb_subset_input_set(self.raw.as_ptr(), sys::HB_SUBSET_SETS_NO_SUBSET_TABLE_TAG) })
    }

    /// The OpenType layout features to keep (fonttools' default list).
    pub fn layout_features(&mut self) -> Set<'_> {
        // SAFETY: as for `unicodes`.
        Set::new(unsafe { sys::hb_subset_input_set(self.raw.as_ptr(), sys::HB_SUBSET_SETS_LAYOUT_FEATURE_TAG) })
    }

    /// Pins every axis of `face` to its default; false if HarfBuzz refuses.
    pub fn pin_all_axes_to_default(&mut self, face: &Face) -> bool {
        // SAFETY: both objects are live; HarfBuzz reads the face's fvar and
        // records the values in the input.
        unsafe { sys::hb_subset_input_pin_all_axes_to_default(self.raw.as_ptr(), face.raw.as_ptr()) != 0 }
    }

    /// Pins axis `tag` of `face` to `value`; false if HarfBuzz refuses.
    pub fn pin_axis_location(&mut self, face: &Face, tag: u32, value: f32) -> bool {
        // SAFETY: as for `pin_all_axes_to_default`.
        unsafe { sys::hb_subset_input_pin_axis_location(self.raw.as_ptr(), face.raw.as_ptr(), tag, value) != 0 }
    }
}

impl Drop for Input {
    fn drop(&mut self) {
        // SAFETY: `raw` holds the reference `hb_subset_input_create_or_fail`
        // returned.
        unsafe { sys::hb_subset_input_destroy(self.raw.as_ptr()) }
    }
}

/// One of an [`Input`]'s sets, borrowed from it.
pub struct Set<'i> {
    raw: *mut sys::hb_set_t,
    _input: PhantomData<&'i mut Input>,
}

impl Set<'_> {
    fn new(raw: *mut sys::hb_set_t) -> Self {
        Self { raw, _input: PhantomData }
    }

    /// Adds `value`.
    pub fn add(&mut self, value: u32) {
        // SAFETY: the set is live while the input is borrowed. HarfBuzz
        // never returns null here: a failed input hands out the inert empty
        // set, on which adds do nothing.
        unsafe { sys::hb_set_add(self.raw, value) }
    }

    /// Removes `value`.
    pub fn remove(&mut self, value: u32) {
        // SAFETY: as for `add`.
        unsafe { sys::hb_set_del(self.raw, value) }
    }

    /// Adds `first..=last`.
    pub fn add_range(&mut self, first: u32, last: u32) {
        // SAFETY: as for `add`.
        unsafe { sys::hb_set_add_range(self.raw, first, last) }
    }

    /// Makes the set hold every value.
    pub fn fill(&mut self) {
        // SAFETY: as for `add`. An empty set inverted holds every value.
        unsafe {
            sys::hb_set_clear(self.raw);
            sys::hb_set_invert(self.raw);
        }
    }
}
