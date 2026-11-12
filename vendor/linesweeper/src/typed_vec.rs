/// Implement methods for vectors indexed with typed indices.
///
/// We use a lot of indices into arrays: we have arrays indexed by input
/// segments, arrays indexed by output segments, arrays indexed by output
/// endpoints, and more. To help keep these straight, we wrap indices
/// in new types, like `SegIdx`, `OutputSegIdx`, etc., and we wrap `Vec`
/// in new types, like `SegVec`, `OutputSegVec`, etc. Then you can only
/// use the right index type with the right vector type.
///
/// This macro implements a bunch of useful methods on the new-type
/// wrappers. It assumes that the index new-type is just a wrapper
/// around `usize`.
macro_rules! impl_typed_vec {
    ($vec_name:ident, $idx_name:ident, $dbg_prefix:expr_2021) => {
        impl std::fmt::Debug for $idx_name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}_{}", $dbg_prefix, self.0)
            }
        }

        #[allow(dead_code)]
        impl<T> $vec_name<T> {
            /// Wraps a plain vector.
            pub fn from_vec(vec: Vec<T>) -> Self {
                Self { inner: vec }
            }

            /// Creates a new vector with capacity for at least `cap` output segments before reallocating.
            pub fn with_capacity(cap: usize) -> Self {
                Self {
                    inner: Vec::with_capacity(cap),
                }
            }

            /// Returns an iterator over all indices into this vector.
            pub fn indices(&self) -> impl Iterator<Item = $idx_name> + use<T> {
                (0..self.inner.len()).map($idx_name)
            }

            /// The length of this vector.
            pub fn len(&self) -> usize {
                self.inner.len()
            }

            /// Are we empty?
            pub fn is_empty(&self) -> bool {
                self.inner.is_empty()
            }

            /// Adds a new element, returning its index.
            pub fn push(&mut self, elt: T) -> $idx_name {
                self.inner.push(elt);
                $idx_name(self.len() - 1)
            }

            /// Returns an iterator over indices and elements.
            pub fn iter(&self) -> impl Iterator<Item = ($idx_name, &T)> + '_ {
                self.inner
                    .iter()
                    .enumerate()
                    .map(|(idx, t)| ($idx_name(idx), t))
            }

            /// Returns an iterator over indices and mutable elements.
            pub fn iter_mut(&mut self) -> impl Iterator<Item = ($idx_name, &mut T)> + '_ {
                self.inner
                    .iter_mut()
                    .enumerate()
                    .map(|(idx, t)| ($idx_name(idx), t))
            }
        }

        #[allow(dead_code)]
        impl<T: Default> $vec_name<T> {
            /// Creates a new vector with `size` elements, each initialized with the default value.
            pub fn with_size(size: usize) -> Self {
                Self {
                    inner: std::iter::from_fn(|| Some(T::default()))
                        .take(size)
                        .collect(),
                }
            }
        }

        impl<T> Default for $vec_name<T> {
            fn default() -> Self {
                Self { inner: Vec::new() }
            }
        }

        impl<T> std::ops::Index<$idx_name> for $vec_name<T> {
            type Output = T;

            fn index(&self, index: $idx_name) -> &Self::Output {
                &self.inner[index.0]
            }
        }

        impl<T> std::ops::IndexMut<$idx_name> for $vec_name<T> {
            fn index_mut(&mut self, index: $idx_name) -> &mut T {
                &mut self.inner[index.0]
            }
        }

        impl<T: std::fmt::Debug> std::fmt::Debug for $vec_name<T> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                struct Entry<'a, T> {
                    idx: $idx_name,
                    inner: &'a T,
                }

                impl<T: std::fmt::Debug> std::fmt::Debug for Entry<'_, T> {
                    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        write!(f, "{idx:?}: {inner:?}", idx = self.idx, inner = self.inner,)
                    }
                }

                let mut list = f.debug_list();
                for idx in self.indices() {
                    list.entry(&Entry {
                        idx,
                        inner: &self[idx],
                    });
                }
                list.finish()
            }
        }
    };
}
