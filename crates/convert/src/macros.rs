//! [`convert_types!`](crate::convert_types) and the helper macros it expands through.
//!
//! The helpers walk a declaration one member at a time and collect, for a struct, the pattern
//! that takes the source apart and the fields of the literal that builds the target; for an
//! enum, the match arms. Neither the pattern nor the literal ever has a `..`, which is what
//! makes a member missing from the declaration a compile error.
//!
//! Each of these is one member short, and none of them compiles. A field the source has:
//!
//! ```compile_fail,E0027
//! mod a { pub struct P { pub x: u8, pub y: u8 } }
//! mod b { pub struct P { pub x: u8 } }
//! craft_convert::convert_types! { struct a::P => b::P { x } }
//! ```
//!
//! A field the target has:
//!
//! ```compile_fail,E0063
//! mod a { pub struct P { pub x: u8 } }
//! mod b { pub struct P { pub x: u8, pub y: u8 } }
//! craft_convert::convert_types! { struct &a::P => b::P { x } }
//! ```
//!
//! A field either twin has, in an `A = B` entry:
//!
//! ```compile_fail,E0063
//! mod a { pub struct P { pub x: u8 } }
//! mod b { pub struct P { pub x: u8, pub y: u8 } }
//! craft_convert::convert_types! { struct a::P = b::P { x } }
//! ```
//!
//! A variant the source has:
//!
//! ```compile_fail,E0004
//! mod a { pub enum E { One, Two } }
//! mod b { pub enum E { One } }
//! craft_convert::convert_types! { enum a::E => b::E { One } }
//! ```
//!
//! A variant the target has:
//!
//! ```compile_fail,E0004
//! mod a { pub enum E { One } }
//! mod b { pub enum E { One, Two } }
//! craft_convert::convert_types! { enum a::E => b::E { One } }
//! ```
//!
//! A field inside a variant:
//!
//! ```compile_fail,E0027
//! mod a { pub enum E { One { x: u8, y: u8 } } }
//! mod b { pub enum E { One { x: u8 } } }
//! craft_convert::convert_types! { enum a::E = b::E { One { x } } }
//! ```
//!
//! With the member named, the last one builds:
//!
//! ```
//! mod a { pub enum E { One { x: u8, y: u8 } } }
//! mod b { pub enum E { One { x: u8 } } }
//! craft_convert::convert_types! { enum a::E = b::E { One { x, y => _ = 0 } } }
//! ```

/// Declares conversions between pairs of structs and enums. See the [crate documentation](crate)
/// for the forms an entry and its members take.
///
/// ```
/// mod a { pub struct Size { pub w: f32, pub h: f32, pub dpi: u16 } }
/// mod b { pub struct Size { pub width: f32, pub height: f32, pub scale: f32 } }
///
/// craft_convert::convert_types! {
///     struct a::Size => b::Size {
///         w => width,
///         h => height,
///         // Only `a` has `dpi` and only `b` has `scale`: say so.
///         dpi => _,
///         _ => scale = 1.0,
///     }
/// }
///
/// use craft_convert::Convert;
/// let size: b::Size = a::Size { w: 2.0, h: 3.0, dpi: 96 }.convert();
/// assert_eq!((size.width, size.height, size.scale), (2.0, 3.0, 1.0));
/// ```
#[macro_export]
macro_rules! convert_types {
    (marker = $m:ty; $($rest:tt)*) => {
        $crate::__convert_items! { [$m] $($rest)* }
    };
    ($($rest:tt)*) => {
        $crate::__convert_items! { [()] $($rest)* }
    };
}

/// One entry at a time.
#[doc(hidden)]
#[macro_export]
macro_rules! __convert_items {
    ([$m:ty]) => {};
    ([$m:ty] struct & $s:ty => $d:ty { $($body:tt)* } $($rest:tt)*) => {
        $crate::__convert_fields! { [@struct [ref $m] [$s] [$d]] {} {} $($body)* , }
        $crate::__convert_items! { [$m] $($rest)* }
    };
    ([$m:ty] struct $s:ty => $d:ty { $($body:tt)* } $($rest:tt)*) => {
        $crate::__convert_fields! { [@struct [value $m] [$s] [$d]] {} {} $($body)* , }
        $crate::__convert_items! { [$m] $($rest)* }
    };
    ([$m:ty] struct $s:ty = $d:ty { $($body:tt)* } $($rest:tt)*) => {
        $crate::__convert_twin_fields! { [@twin_struct [$m] [$s] [$d]] {} {} {} {} $($body)* , }
        $crate::__convert_items! { [$m] $($rest)* }
    };
    ([$m:ty] enum & $s:ty => $d:ty { $($body:tt)* } $($rest:tt)*) => {
        $crate::__convert_variants! { [[ref $m] [$s] [$d]] {} {} {} $($body)* , }
        $crate::__convert_items! { [$m] $($rest)* }
    };
    ([$m:ty] enum $s:ty => $d:ty { $($body:tt)* } $($rest:tt)*) => {
        $crate::__convert_variants! { [[value $m] [$s] [$d]] {} {} {} $($body)* , }
        $crate::__convert_items! { [$m] $($rest)* }
    };
    ([$m:ty] enum $s:ty = $d:ty { $($body:tt)* } $($rest:tt)*) => {
        $crate::__convert_twin_variants! { [[$m] [$s] [$d]] {} {} $($body)* , }
        $crate::__convert_items! { [$m] $($rest)* }
    };
}

/// The members of a one-way struct or variant: `{pattern} {literal}`, then on to `$k`.
///
/// A literal entry is `{target conv source}` (convert the bound source field) or
/// `{target expr expression}`.
#[doc(hidden)]
#[macro_export]
macro_rules! __convert_fields {
    ([$($k:tt)*] $pat:tt $lit:tt $(,)?) => {
        $crate::__convert_emit! { $($k)* $pat $lit }
    };
    ($k:tt {$($pat:tt)*} {$($lit:tt)*} _ => $b:ident = $e:expr , $($rest:tt)*) => {
        $crate::__convert_fields! { $k {$($pat)*} {$($lit)* {$b expr $e}} $($rest)* }
    };
    ($k:tt {$($pat:tt)*} {$($lit:tt)*} $a:ident => _ , $($rest:tt)*) => {
        $crate::__convert_fields! { $k {$($pat)* $a: _,} {$($lit)*} $($rest)* }
    };
    ($k:tt {$($pat:tt)*} {$($lit:tt)*} $a:ident => $b:ident = $e:expr , $($rest:tt)*) => {
        $crate::__convert_fields! { $k {$($pat)* $a,} {$($lit)* {$b expr $e}} $($rest)* }
    };
    ($k:tt {$($pat:tt)*} {$($lit:tt)*} $a:ident => $b:ident , $($rest:tt)*) => {
        $crate::__convert_fields! { $k {$($pat)* $a,} {$($lit)* {$b conv $a}} $($rest)* }
    };
    ($k:tt {$($pat:tt)*} {$($lit:tt)*} $a:ident = $e:expr , $($rest:tt)*) => {
        $crate::__convert_fields! { $k {$($pat)* $a,} {$($lit)* {$a expr $e}} $($rest)* }
    };
    ($k:tt {$($pat:tt)*} {$($lit:tt)*} $a:ident , $($rest:tt)*) => {
        $crate::__convert_fields! { $k {$($pat)* $a,} {$($lit)* {$a conv $a}} $($rest)* }
    };
}

/// The members of an `A = B` struct or variant: pattern and literal for each direction.
#[doc(hidden)]
#[macro_export]
macro_rules! __convert_twin_fields {
    ([$($k:tt)*] $fpat:tt $flit:tt $rpat:tt $rlit:tt $(,)?) => {
        $crate::__convert_emit! { $($k)* $fpat $flit $rpat $rlit }
    };
    ($k:tt $fpat:tt {$($flit:tt)*} {$($rpat:tt)*} $rlit:tt _ => $b:ident = $e:expr , $($rest:tt)*) => {
        $crate::__convert_twin_fields! { $k $fpat {$($flit)* {$b expr $e}} {$($rpat)* $b: _,} $rlit $($rest)* }
    };
    ($k:tt {$($fpat:tt)*} $flit:tt $rpat:tt {$($rlit:tt)*} $a:ident => _ = $e:expr , $($rest:tt)*) => {
        $crate::__convert_twin_fields! { $k {$($fpat)* $a: _,} $flit $rpat {$($rlit)* {$a expr $e}} $($rest)* }
    };
    ($k:tt {$($fpat:tt)*} {$($flit:tt)*} {$($rpat:tt)*} {$($rlit:tt)*} $a:ident => $b:ident , $($rest:tt)*) => {
        $crate::__convert_twin_fields! { $k {$($fpat)* $a,} {$($flit)* {$b conv $a}} {$($rpat)* $b,} {$($rlit)* {$a conv $b}} $($rest)* }
    };
    ($k:tt {$($fpat:tt)*} {$($flit:tt)*} {$($rpat:tt)*} {$($rlit:tt)*} $a:ident = $e:expr , $($rest:tt)*) => {
        $crate::__convert_twin_fields! { $k {$($fpat)* $a,} {$($flit)* {$a expr $e}} {$($rpat)* $a,} {$($rlit)* {$a expr $e}} $($rest)* }
    };
    ($k:tt {$($fpat:tt)*} {$($flit:tt)*} {$($rpat:tt)*} {$($rlit:tt)*} $a:ident , $($rest:tt)*) => {
        $crate::__convert_twin_fields! { $k {$($fpat)* $a,} {$($flit)* {$a conv $a}} {$($rpat)* $a,} {$($rlit)* {$a conv $a}} $($rest)* }
    };
}

/// The variants of a one-way enum: `{arms} {target-only variants} {arms with their own expression}`.
///
/// An arm is `{Source [pattern] Target kind payload}`.
#[doc(hidden)]
#[macro_export]
macro_rules! __convert_variants {
    ([$($k:tt)*] $arms:tt $only:tt $own:tt $(,)?) => {
        $crate::__convert_emit! { @enum $($k)* $arms $only $own }
    };
    ($k:tt $arms:tt {$($only:tt)*} $own:tt _ => $w:ident , $($rest:tt)*) => {
        $crate::__convert_variants! { $k $arms {$($only)* $w} $own $($rest)* }
    };
    ($k:tt $arms:tt $only:tt {$($own:tt)*} $v:ident { $($p:tt)* } = $e:expr , $($rest:tt)*) => {
        $crate::__convert_variants! { $k $arms $only {$($own)* __ConvertSource::$v { $($p)* } => $e,} $($rest)* }
    };
    ($k:tt $arms:tt $only:tt $own:tt $v:ident { $($f:tt)* } => $w:ident , $($rest:tt)*) => {
        $crate::__convert_fields! { [@variant $k $arms $only $own [$v $w] [$($rest)*]] {} {} $($f)* , }
    };
    ($k:tt $arms:tt $only:tt $own:tt $v:ident { $($f:tt)* } , $($rest:tt)*) => {
        $crate::__convert_fields! { [@variant $k $arms $only $own [$v $v] [$($rest)*]] {} {} $($f)* , }
    };
    ($k:tt $arms:tt $only:tt {$($own:tt)*} $v:ident ( $($p:tt)* ) = $e:expr , $($rest:tt)*) => {
        $crate::__convert_variants! { $k $arms $only {$($own)* __ConvertSource::$v($($p)*) => $e,} $($rest)* }
    };
    ($k:tt {$($arms:tt)*} $only:tt $own:tt $v:ident ( $($a:ident),* $(,)? ) => $w:ident , $($rest:tt)*) => {
        $crate::__convert_variants! { $k {$($arms)* {$v [($($a),*)] $w tuple ($($a)*)}} $only $own $($rest)* }
    };
    ($k:tt {$($arms:tt)*} $only:tt $own:tt $v:ident ( $($a:ident),* $(,)? ) , $($rest:tt)*) => {
        $crate::__convert_variants! { $k {$($arms)* {$v [($($a),*)] $v tuple ($($a)*)}} $only $own $($rest)* }
    };
    ($k:tt $arms:tt $only:tt {$($own:tt)*} $v:ident = $e:expr , $($rest:tt)*) => {
        $crate::__convert_variants! { $k $arms $only {$($own)* __ConvertSource::$v => $e,} $($rest)* }
    };
    ($k:tt {$($arms:tt)*} $only:tt $own:tt $v:ident => $w:ident , $($rest:tt)*) => {
        $crate::__convert_variants! { $k {$($arms)* {$v [] $w unit ()}} $only $own $($rest)* }
    };
    ($k:tt {$($arms:tt)*} $only:tt $own:tt $v:ident , $($rest:tt)*) => {
        $crate::__convert_variants! { $k {$($arms)* {$v [] $v unit ()}} $only $own $($rest)* }
    };
}

/// The variants of an `A = B` enum: the arms of each direction.
#[doc(hidden)]
#[macro_export]
macro_rules! __convert_twin_variants {
    ([[$m:ty] [$s:ty] [$d:ty]] $farms:tt $rarms:tt $(,)?) => {
        $crate::__convert_emit! { @enum [value $m] [$s] [$d] $farms {} {} }
        $crate::__convert_emit! { @enum [ref $m] [$s] [$d] $farms {} {} }
        $crate::__convert_emit! { @enum [value $m] [$d] [$s] $rarms {} {} }
        $crate::__convert_emit! { @enum [ref $m] [$d] [$s] $rarms {} {} }
    };
    ($k:tt $farms:tt $rarms:tt $v:ident { $($f:tt)* } => $w:ident , $($rest:tt)*) => {
        $crate::__convert_twin_fields! { [@twin_variant $k $farms $rarms [$v $w] [$($rest)*]] {} {} {} {} $($f)* , }
    };
    ($k:tt $farms:tt $rarms:tt $v:ident { $($f:tt)* } , $($rest:tt)*) => {
        $crate::__convert_twin_fields! { [@twin_variant $k $farms $rarms [$v $v] [$($rest)*]] {} {} {} {} $($f)* , }
    };
    ($k:tt {$($farms:tt)*} {$($rarms:tt)*} $v:ident ( $($a:ident),* $(,)? ) => $w:ident , $($rest:tt)*) => {
        $crate::__convert_twin_variants! { $k {$($farms)* {$v [($($a),*)] $w tuple ($($a)*)}} {$($rarms)* {$w [($($a),*)] $v tuple ($($a)*)}} $($rest)* }
    };
    ($k:tt {$($farms:tt)*} {$($rarms:tt)*} $v:ident ( $($a:ident),* $(,)? ) , $($rest:tt)*) => {
        $crate::__convert_twin_variants! { $k {$($farms)* {$v [($($a),*)] $v tuple ($($a)*)}} {$($rarms)* {$v [($($a),*)] $v tuple ($($a)*)}} $($rest)* }
    };
    ($k:tt {$($farms:tt)*} {$($rarms:tt)*} $v:ident => $w:ident , $($rest:tt)*) => {
        $crate::__convert_twin_variants! { $k {$($farms)* {$v [] $w unit ()}} {$($rarms)* {$w [] $v unit ()}} $($rest)* }
    };
    ($k:tt {$($farms:tt)*} {$($rarms:tt)*} $v:ident , $($rest:tt)*) => {
        $crate::__convert_twin_variants! { $k {$($farms)* {$v [] $v unit ()}} {$($rarms)* {$v [] $v unit ()}} $($rest)* }
    };
}

/// Writes the impls, and carries a finished variant back to the enum it belongs to.
#[doc(hidden)]
#[macro_export]
macro_rules! __convert_emit {
    (@struct [value $m:ty] [$s:ty] [$d:ty] {$($pat:tt)*} {$({$b:ident $kind:ident $x:tt})*}) => {
        impl $crate::Convert<$d, $m> for $s {
            fn convert(self) -> $d {
                type __ConvertSource = $s;
                type __ConvertTarget = $d;
                let __ConvertSource { $($pat)* } = self;
                __ConvertTarget { $($b: $crate::__convert_value!([value $m] $kind $x),)* }
            }
        }
    };
    (@struct [ref $m:ty] [$s:ty] [$d:ty] {$($pat:tt)*} {$({$b:ident $kind:ident $x:tt})*}) => {
        impl $crate::ConvertRef<$d, $m> for $s {
            fn convert_ref(&self) -> $d {
                type __ConvertSource = $s;
                type __ConvertTarget = $d;
                let __ConvertSource { $($pat)* } = self;
                __ConvertTarget { $($b: $crate::__convert_value!([ref $m] $kind $x),)* }
            }
        }
    };
    (@twin_struct [$m:ty] [$s:ty] [$d:ty] $fpat:tt $flit:tt $rpat:tt $rlit:tt) => {
        $crate::__convert_emit! { @struct [value $m] [$s] [$d] $fpat $flit }
        $crate::__convert_emit! { @struct [ref $m] [$s] [$d] $fpat $flit }
        $crate::__convert_emit! { @struct [value $m] [$d] [$s] $rpat $rlit }
        $crate::__convert_emit! { @struct [ref $m] [$d] [$s] $rpat $rlit }
    };
    (@enum [value $m:ty] [$s:ty] [$d:ty] {$({$v:ident [$($sp:tt)*] $w:ident $kind:ident $payload:tt})*} {$($only:ident)*} {$($own:tt)*}) => {
        impl $crate::Convert<$d, $m> for $s {
            fn convert(self) -> $d {
                type __ConvertSource = $s;
                type __ConvertTarget = $d;
                // Not called: it fails to compile when the target has a variant this entry does not name.
                #[allow(dead_code, unreachable_patterns)]
                fn __convert_covers_target(target: &__ConvertTarget) {
                    match *target {
                        $(__ConvertTarget::$w { .. } => (),)*
                        $(__ConvertTarget::$only { .. } => (),)*
                    }
                }
                match self {
                    $(__ConvertSource::$v $($sp)* => $crate::__convert_arm!([value $m] $w $kind $payload),)*
                    $($own)*
                }
            }
        }
    };
    (@enum [ref $m:ty] [$s:ty] [$d:ty] {$({$v:ident [$($sp:tt)*] $w:ident $kind:ident $payload:tt})*} {$($only:ident)*} {$($own:tt)*}) => {
        impl $crate::ConvertRef<$d, $m> for $s {
            fn convert_ref(&self) -> $d {
                type __ConvertSource = $s;
                type __ConvertTarget = $d;
                // Not called: it fails to compile when the target has a variant this entry does not name.
                #[allow(dead_code, unreachable_patterns)]
                fn __convert_covers_target(target: &__ConvertTarget) {
                    match *target {
                        $(__ConvertTarget::$w { .. } => (),)*
                        $(__ConvertTarget::$only { .. } => (),)*
                    }
                }
                match self {
                    $(__ConvertSource::$v $($sp)* => $crate::__convert_arm!([ref $m] $w $kind $payload),)*
                    $($own)*
                }
            }
        }
    };
    (@variant $k:tt {$($arms:tt)*} $only:tt $own:tt [$v:ident $w:ident] [$($rest:tt)*] {$($pat:tt)*} $lit:tt) => {
        $crate::__convert_variants! { $k {$($arms)* {$v [{ $($pat)* }] $w named $lit}} $only $own $($rest)* }
    };
    (@twin_variant $k:tt {$($farms:tt)*} {$($rarms:tt)*} [$v:ident $w:ident] [$($rest:tt)*] {$($fpat:tt)*} $flit:tt {$($rpat:tt)*} $rlit:tt) => {
        $crate::__convert_twin_variants! { $k {$($farms)* {$v [{ $($fpat)* }] $w named $flit}} {$($rarms)* {$w [{ $($rpat)* }] $v named $rlit}} $($rest)* }
    };
}

/// The value a match arm builds.
#[doc(hidden)]
#[macro_export]
macro_rules! __convert_arm {
    ($mode:tt $w:ident unit ()) => {
        __ConvertTarget::$w
    };
    ($mode:tt $w:ident tuple ($($a:ident)*)) => {
        __ConvertTarget::$w($($crate::__convert_value!($mode conv $a)),*)
    };
    ($mode:tt $w:ident named {$({$b:ident $kind:ident $x:tt})*}) => {
        __ConvertTarget::$w { $($b: $crate::__convert_value!($mode $kind $x),)* }
    };
}

/// One member's value: the converted source field, or the declaration's own expression.
#[doc(hidden)]
#[macro_export]
macro_rules! __convert_value {
    ([value $m:ty] conv $a:ident) => {
        $crate::Convert::<_, $m>::convert($a)
    };
    ([ref $m:ty] conv $a:ident) => {
        $crate::ConvertRef::<_, $m>::convert_ref($a)
    };
    ($mode:tt expr $e:expr) => {
        $e
    };
}
