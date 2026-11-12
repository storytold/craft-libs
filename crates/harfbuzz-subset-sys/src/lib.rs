//! The HarfBuzz subsetter, built from vendored source. Native only: `build.rs` compiles HarfBuzz's C++ with the
//! target's C++ compiler.
//!
//! The crate root holds raw declarations for the calls craft-font-subset makes, taken from the vendored `hb-blob.h`,
//! `hb-face.h`, `hb-set.h` and `hb-subset.h`; [`safe`] wraps them. The library itself is
//! `harfbuzz/src/harfbuzz-subset.cc`, compiled by `build.rs`.
//!
//! This crate, craft-win32 and the benchmark's allocator are the places that allow `unsafe`; the design-lint rule
//! `contract/unsafe-allow` names them. Every block in [`safe`] says why it is sound.

#![no_std]
// The raw declarations mirror HarfBuzz's headers, which document them; `safe` keeps the lint.
#![allow(unsafe_code, non_camel_case_types, missing_docs)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

extern crate alloc;

pub mod safe;

use core::ffi::{c_char, c_int, c_uint, c_void};
use core::marker::{PhantomData, PhantomPinned};

macro_rules! opaque {
    ($($name:ident),*) => {$(
        #[repr(C)]
        pub struct $name {
            _data: [u8; 0],
            _marker: PhantomData<(*mut u8, PhantomPinned)>,
        }
    )*};
}

opaque!(hb_blob_t, hb_face_t, hb_set_t, hb_subset_input_t);

pub type hb_bool_t = c_int;
pub type hb_codepoint_t = u32;
pub type hb_tag_t = u32;
pub type hb_memory_mode_t = c_uint;
pub type hb_subset_sets_t = c_uint;
pub type hb_destroy_func_t = Option<unsafe extern "C" fn(user_data: *mut c_void)>;

pub const HB_MEMORY_MODE_READONLY: hb_memory_mode_t = 1;

pub const HB_SUBSET_FLAGS_NO_HINTING: c_uint = 0x1;
pub const HB_SUBSET_FLAGS_RETAIN_GIDS: c_uint = 0x2;

pub const HB_SUBSET_SETS_NO_SUBSET_TABLE_TAG: hb_subset_sets_t = 2;
pub const HB_SUBSET_SETS_DROP_TABLE_TAG: hb_subset_sets_t = 3;
pub const HB_SUBSET_SETS_NAME_ID: hb_subset_sets_t = 4;
pub const HB_SUBSET_SETS_LAYOUT_FEATURE_TAG: hb_subset_sets_t = 6;

unsafe extern "C" {
    pub fn hb_blob_create_or_fail(
        data: *const c_char,
        length: c_uint,
        mode: hb_memory_mode_t,
        user_data: *mut c_void,
        destroy: hb_destroy_func_t,
    ) -> *mut hb_blob_t;
    pub fn hb_blob_destroy(blob: *mut hb_blob_t);
    pub fn hb_blob_get_data(blob: *mut hb_blob_t, length: *mut c_uint) -> *const c_char;

    pub fn hb_face_create(blob: *mut hb_blob_t, index: c_uint) -> *mut hb_face_t;
    pub fn hb_face_destroy(face: *mut hb_face_t);
    pub fn hb_face_reference_blob(face: *mut hb_face_t) -> *mut hb_blob_t;

    pub fn hb_set_add(set: *mut hb_set_t, codepoint: hb_codepoint_t);
    pub fn hb_set_del(set: *mut hb_set_t, codepoint: hb_codepoint_t);
    pub fn hb_set_add_range(set: *mut hb_set_t, first: hb_codepoint_t, last: hb_codepoint_t);
    pub fn hb_set_clear(set: *mut hb_set_t);
    pub fn hb_set_invert(set: *mut hb_set_t);

    pub fn hb_subset_input_create_or_fail() -> *mut hb_subset_input_t;
    pub fn hb_subset_input_destroy(input: *mut hb_subset_input_t);
    pub fn hb_subset_input_get_flags(input: *mut hb_subset_input_t) -> c_uint;
    pub fn hb_subset_input_set_flags(input: *mut hb_subset_input_t, value: c_uint);
    pub fn hb_subset_input_unicode_set(input: *mut hb_subset_input_t) -> *mut hb_set_t;
    pub fn hb_subset_input_set(input: *mut hb_subset_input_t, set_type: hb_subset_sets_t) -> *mut hb_set_t;
    pub fn hb_subset_input_pin_all_axes_to_default(input: *mut hb_subset_input_t, face: *mut hb_face_t) -> hb_bool_t;
    pub fn hb_subset_input_pin_axis_location(input: *mut hb_subset_input_t, face: *mut hb_face_t, axis_tag: hb_tag_t, axis_value: f32) -> hb_bool_t;

    pub fn hb_subset_or_fail(source: *mut hb_face_t, input: *const hb_subset_input_t) -> *mut hb_face_t;
}
