//! Reading bytes and PDF objects from data.

use crate::object::ObjectIdentifier;
use crate::sync::Arc;
use crate::trivia::{Comment, is_eol_character, is_white_space_character};
use crate::xref::XRef;
use smallvec::{SmallVec, smallvec};

pub use crate::byte_reader::Reader;

/// Extension trait for the `Reader` struct.
pub trait ReaderExt<'a> {
    fn read<T: Readable<'a>>(&mut self, ctx: &ReaderContext<'a>) -> Option<T>;
    fn read_with_context<T: Readable<'a>>(&mut self, ctx: &ReaderContext<'a>) -> Option<T>;
    fn read_without_context<T: Readable<'a>>(&mut self) -> Option<T>;
    fn skip<T: Skippable>(&mut self, is_content_stream: bool) -> Option<&'a [u8]>;
    fn skip_white_spaces(&mut self);
    fn read_white_space(&mut self) -> Option<()>;
    fn skip_eol_characters(&mut self);
    fn skip_white_spaces_and_comments(&mut self);
}

impl<'a> ReaderExt<'a> for Reader<'a> {
    // Note: If `PLAIN` is true, it means that the data we are about to read _might_ contain
    // an object reference instead of an actual object. if `PLAIN` is false, then an object
    // reference cannot occur. The main reason we make this distinction is that when parsing
    // a number, we cannot unambiguously distinguish whether it's a real number or the
    // start of an object reference. In content streams, object references cannot appear,
    // so in order to speed this up we set `PLAIN` to false, meaning that as soon as we
    // encounter a number we know it's a number, and don't need to do a look-ahead to ensure
    // that it's not an object reference.
    #[inline]
    fn read<T: Readable<'a>>(&mut self, ctx: &ReaderContext<'a>) -> Option<T> {
        crate::cancel::check(ctx.control())?;
        self.set_control(ctx.control().cloned());
        let old_offset = self.offset;

        T::read(self, ctx).or_else(|| {
            self.offset = old_offset;

            None
        })
    }

    #[inline]
    fn read_with_context<T: Readable<'a>>(&mut self, ctx: &ReaderContext<'a>) -> Option<T> {
        self.read::<T>(ctx)
    }

    #[inline]
    fn read_without_context<T: Readable<'a>>(&mut self) -> Option<T> {
        self.read::<T>(ReaderContext::dummy_ref())
    }

    #[inline]
    fn skip<T: Skippable>(&mut self, is_content_stream: bool) -> Option<&'a [u8]> {
        let old_offset = self.offset;

        T::skip(self, is_content_stream).or_else(|| {
            self.offset = old_offset;
            None
        })?;

        self.data.get(old_offset..self.offset)
    }

    #[inline]
    fn skip_white_spaces(&mut self) {
        while let Some(b) = self.peek_byte() {
            if is_white_space_character(b) {
                self.forward();
            } else {
                return;
            }
        }
    }

    #[inline]
    fn read_white_space(&mut self) -> Option<()> {
        if is_white_space_character(self.peek_byte()?) {
            let w = self.read_byte()?;

            if w == b'\r' && self.peek_byte().is_some_and(|b| b == b'\n') {
                self.read_byte()?;
            }

            return Some(());
        }

        None
    }

    #[inline]
    fn skip_eol_characters(&mut self) {
        while let Some(b) = self.peek_byte() {
            if is_eol_character(b) {
                self.forward();
            } else {
                return;
            }
        }
    }

    #[inline]
    fn skip_white_spaces_and_comments(&mut self) {
        while let Some(b) = self.peek_byte() {
            if is_white_space_character(b) {
                self.skip_white_spaces();
            } else if b == b'%' {
                Comment::skip(self, true);
            } else {
                return;
            }
        }
    }
}

#[derive(Clone, Debug)]
struct ReaderContextData<'a> {
    xref: &'a XRef,
    in_content_stream: bool,
    in_object_stream: bool,
    obj_number: Option<ObjectIdentifier>,
    parent_chain: SmallVec<[ObjectIdentifier; 8]>,
}

#[derive(Clone, Debug)]
enum ReaderContextInner<'a> {
    Shared(Arc<ReaderContextData<'a>>),
    Dummy { in_content_stream: bool, limits: crate::DecodeLimits },
    ControlledDummy(Arc<ControlledDummy>),
}

#[derive(Clone, Debug)]
struct ControlledDummy {
    in_content_stream: bool,
    limits: crate::DecodeLimits,
    control: crate::cancel::Control,
}

/// Context for reading PDF objects.
#[derive(Clone, Debug)]
pub struct ReaderContext<'a>(ReaderContextInner<'a>);

impl<'a> ReaderContext<'a> {
    pub(crate) fn new(xref: &'a XRef, in_content_stream: bool) -> Self {
        Self(ReaderContextInner::Shared(Arc::new(ReaderContextData {
            xref,
            in_content_stream,
            obj_number: None,
            in_object_stream: false,
            parent_chain: smallvec![],
        })))
    }

    pub fn dummy() -> Self {
        Self::dummy_with_limits(crate::DecodeLimits::default())
    }

    pub(crate) fn dummy_with_limits(limits: crate::DecodeLimits) -> Self {
        Self(ReaderContextInner::Dummy { in_content_stream: false, limits })
    }

    pub(crate) fn dummy_with_control(limits: crate::DecodeLimits, control: Option<crate::cancel::Control>) -> Self {
        match control {
            Some(control) => Self(ReaderContextInner::ControlledDummy(Arc::new(ControlledDummy { in_content_stream: false, limits, control }))),
            None => Self::dummy_with_limits(limits),
        }
    }

    pub(crate) fn control(&self) -> Option<&crate::cancel::Control> {
        match &self.0 {
            ReaderContextInner::Shared(inner) => inner.xref.control(),
            ReaderContextInner::Dummy { .. } => None,
            ReaderContextInner::ControlledDummy(inner) => Some(&inner.control).filter(|c| c.active()),
        }
    }

    pub(crate) fn decode_limits(&self) -> crate::DecodeLimits {
        match &self.0 {
            ReaderContextInner::Shared(inner) => inner.xref.decode_limits(),
            ReaderContextInner::Dummy { limits, .. } => *limits,
            ReaderContextInner::ControlledDummy(inner) => inner.limits,
        }
    }

    pub(crate) fn dummy_ref() -> &'static Self {
        &Self(ReaderContextInner::Dummy { in_content_stream: true, limits: crate::DecodeLimits::DEFAULT })
    }

    #[inline]
    pub(crate) fn xref(&self) -> &'a XRef {
        match &self.0 {
            ReaderContextInner::Shared(inner) => inner.xref,
            ReaderContextInner::Dummy { .. } | ReaderContextInner::ControlledDummy(_) => XRef::dummy(),
        }
    }

    #[inline]
    pub(crate) fn in_content_stream(&self) -> bool {
        match &self.0 {
            ReaderContextInner::Shared(inner) => inner.in_content_stream,
            ReaderContextInner::Dummy { in_content_stream, .. } => *in_content_stream,
            ReaderContextInner::ControlledDummy(inner) => inner.in_content_stream,
        }
    }

    #[inline]
    pub(crate) fn in_object_stream(&self) -> bool {
        match &self.0 {
            ReaderContextInner::Shared(inner) => inner.in_object_stream,
            ReaderContextInner::Dummy { .. } | ReaderContextInner::ControlledDummy(_) => false,
        }
    }

    #[inline]
    pub(crate) fn obj_number(&self) -> Option<ObjectIdentifier> {
        match &self.0 {
            ReaderContextInner::Shared(inner) => inner.obj_number,
            ReaderContextInner::Dummy { .. } | ReaderContextInner::ControlledDummy(_) => None,
        }
    }

    #[inline]
    pub(crate) fn set_obj_number(&mut self, id: ObjectIdentifier) {
        match &mut self.0 {
            ReaderContextInner::Shared(inner) => Arc::make_mut(inner).obj_number = Some(id),
            ReaderContextInner::Dummy { .. } | ReaderContextInner::ControlledDummy(_) => {}
        }
    }

    #[inline]
    pub(crate) fn set_in_content_stream(&mut self, val: bool) {
        match &mut self.0 {
            ReaderContextInner::Shared(inner) => Arc::make_mut(inner).in_content_stream = val,
            ReaderContextInner::Dummy { in_content_stream, .. } => *in_content_stream = val,
            ReaderContextInner::ControlledDummy(inner) => Arc::make_mut(inner).in_content_stream = val,
        }
    }

    #[inline]
    pub(crate) fn set_in_object_stream(&mut self, val: bool) {
        match &mut self.0 {
            ReaderContextInner::Shared(inner) => Arc::make_mut(inner).in_object_stream = val,
            ReaderContextInner::Dummy { .. } | ReaderContextInner::ControlledDummy(_) => {}
        }
    }

    #[inline]
    pub(crate) fn parent_chain_contains(&self, id: &ObjectIdentifier) -> bool {
        match &self.0 {
            ReaderContextInner::Shared(inner) => inner.parent_chain.contains(id),
            ReaderContextInner::Dummy { .. } | ReaderContextInner::ControlledDummy(_) => false,
        }
    }

    #[inline]
    pub(crate) fn parent_chain_push(&mut self, id: ObjectIdentifier) {
        match &mut self.0 {
            ReaderContextInner::Shared(inner) => Arc::make_mut(inner).parent_chain.push(id),
            ReaderContextInner::Dummy { .. } | ReaderContextInner::ControlledDummy(_) => {}
        }
    }
}

pub trait Readable<'a>: Sized {
    fn read(r: &mut Reader<'a>, ctx: &ReaderContext<'a>) -> Option<Self>;
    fn from_bytes_impl(b: &'a [u8]) -> Option<Self> {
        let mut r = Reader::new(b);
        Self::read(&mut r, &ReaderContext::dummy())
    }
}

pub trait Skippable {
    fn skip(r: &mut Reader<'_>, is_content_stream: bool) -> Option<()>;
}
