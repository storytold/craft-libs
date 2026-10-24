//! What a device says it can do, as plain data.
//!
//! A device (a printer, a scanner, anything a person configures from a dialog) answers questions
//! about an option in one of three ways, and a dialog must treat them differently:
//!
//! | State | Meaning | A dialog should |
//! |---|---|---|
//! | [`Capability::Supported`] | the device published a list or a range | enable the control and offer only that |
//! | [`Capability::Unsupported`] | the device published its answer and the option is not in it | show the control disabled, with a reason, and send nothing |
//! | [`Capability::Unknown`] | the device did not say, or asking failed | enable the control, say it is not confirmed, and send the value as a hint |
//!
//! "Unknown" is not "unsupported": options are hints the backend may ignore, so hiding a control
//! the device might honour would be wrong. Conversely, offering a value the device told us it does
//! not have would be a lie. This type keeps the two apart.
//!
//! The type is capability only (does a duplex unit exist, which sizes are listed). A device's
//! dynamic status (out of paper, cover open) is a different thing and never lives here.
//!
//! The types hold no wording: the reason shown for [`Capability::Unsupported`] and the note for
//! [`Capability::Unknown`] belong to the application. They have no dependency on egui.

/// Which values a device offers.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Choices<T> {
    /// A list, in the order the application should offer it.
    List(Vec<T>),
    /// An inclusive range.
    Range { min: T, max: T },
}

/// A published answer: what is offered, and which value the device uses when none is asked for.
///
/// Not constructible by struct literal outside this crate (`#[non_exhaustive]`), so outside code goes
/// through the constructors on [`Capability`]. The fields stay public and mutable.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Advertised<T> {
    pub choices: Choices<T>,
    /// The device's own default, when it says. For a list built by
    /// [`Capability::list_default_first`] it is also the first entry.
    pub default: Option<T>,
}

/// The three states of one option. See the [module documentation](self).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Capability<T> {
    /// The device did not publish this option (or the query failed).
    Unknown,
    /// The device published what it offers.
    Supported(Advertised<T>),
    /// The device published its answer and this option is not part of it. Carries no reason: the
    /// application words it.
    Unsupported,
}

// Not derived: a derive would require `T: Default`, which an option value need not be.
#[allow(clippy::derivable_impls)]
impl<T> Default for Capability<T> {
    fn default() -> Self {
        Capability::Unknown
    }
}

impl<T> Capability<T> {
    /// `None` is unknown, an empty list is unsupported, a non-empty list is supported.
    pub fn from_option_list(list: Option<&[T]>) -> Self
    where
        T: Clone,
    {
        match list {
            None => Capability::Unknown,
            Some([]) => Capability::Unsupported,
            Some(values) => Capability::Supported(Advertised { choices: Choices::List(values.to_vec()), default: None }),
        }
    }

    /// `None` is unknown; a range whose minimum is above its maximum is unsupported.
    pub fn from_option_range(range: Option<(T, T)>) -> Self
    where
        T: PartialOrd,
    {
        match range {
            None => Capability::Unknown,
            Some((min, max)) if min > max => Capability::Unsupported,
            Some((min, max)) => Capability::Supported(Advertised { choices: Choices::Range { min, max }, default: None }),
        }
    }

    /// A published list with the device default moved to the front. A default the list does not
    /// contain is ignored (it is not an offered value). An empty list is unsupported.
    pub fn list_default_first(list: &[T], default: Option<&T>) -> Self
    where
        T: Clone + PartialEq,
    {
        if list.is_empty() {
            return Capability::Unsupported;
        }
        let default = default.filter(|d| list.contains(d)).cloned();
        let mut values: Vec<T> = Vec::with_capacity(list.len());
        if let Some(d) = &default {
            values.push(d.clone());
        }
        values.extend(list.iter().filter(|v| default.as_ref() != Some(*v)).cloned());
        Capability::Supported(Advertised { choices: Choices::List(values), default })
    }

    /// Record the device default of a supported option. It must be offered (in the list, or inside
    /// the range) or it is ignored; the other states are returned unchanged. The list order is kept
    /// (use [`Self::list_default_first`] to reorder).
    #[must_use]
    pub fn with_default(mut self, default: T) -> Self
    where
        T: PartialOrd,
    {
        if self.allows(&default)
            && let Capability::Supported(a) = &mut self
        {
            a.default = Some(default);
        }
        self
    }

    /// The device said something (supported or unsupported).
    pub fn is_known(&self) -> bool {
        !matches!(self, Capability::Unknown)
    }

    /// The device listed the option ([`Capability::Supported`]).
    pub fn is_supported(&self) -> bool {
        matches!(self, Capability::Supported(_))
    }

    /// The device said it does not take the option ([`Capability::Unsupported`]).
    pub fn is_unsupported(&self) -> bool {
        matches!(self, Capability::Unsupported)
    }

    /// Should the control be enabled? True for supported and for unknown.
    pub fn is_enabled(&self) -> bool {
        !self.is_unsupported()
    }

    /// What is offered, when the device published it.
    pub fn choices(&self) -> Option<&Choices<T>> {
        match self {
            Capability::Supported(a) => Some(&a.choices),
            _ => None,
        }
    }

    /// The listed values (empty for a range and for the other states).
    pub fn list(&self) -> &[T] {
        match self {
            Capability::Supported(Advertised { choices: Choices::List(values), .. }) => values,
            _ => &[],
        }
    }

    /// The device default, when published.
    pub fn default_value(&self) -> Option<&T> {
        match self {
            Capability::Supported(a) => a.default.as_ref(),
            _ => None,
        }
    }

    /// May `value` be sent? Supported: only when listed or in range. Unknown: yes, as a hint.
    /// Unsupported: never.
    pub fn allows(&self, value: &T) -> bool
    where
        T: PartialOrd,
    {
        match self {
            Capability::Unknown => true,
            Capability::Unsupported => false,
            Capability::Supported(a) => match &a.choices {
                Choices::List(values) => values.contains(value),
                Choices::Range { min, max } => min <= value && value <= max,
            },
        }
    }

    /// The value to send for `value`: unknown keeps it (a hint); unsupported drops it; a list keeps
    /// it only when listed; a range moves it into the range.
    pub fn clamp_to_supported(&self, value: &T) -> Option<T>
    where
        T: PartialOrd + Clone,
    {
        match self {
            Capability::Unknown => Some(value.clone()),
            Capability::Unsupported => None,
            Capability::Supported(a) => match &a.choices {
                Choices::List(values) => values.contains(value).then(|| value.clone()),
                Choices::Range { min, max } => Some(if value < min {
                    min.clone()
                } else if value > max {
                    max.clone()
                } else {
                    value.clone()
                }),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_list_is_unknown_an_empty_one_unsupported_and_a_full_one_supported() {
        let none: Capability<u32> = Capability::from_option_list(None);
        assert_eq!(none, Capability::Unknown);
        let empty: Capability<u32> = Capability::from_option_list(Some(&[]));
        assert_eq!(empty, Capability::Unsupported);
        let some = Capability::from_option_list(Some(&[1u32, 2][..]));
        assert_eq!(some.list(), [1, 2]);
        assert!(some.is_supported() && some.is_known() && some.is_enabled());
        assert_eq!(some.default_value(), None);
        assert_eq!(Capability::<u32>::default(), Capability::Unknown);
    }

    #[test]
    fn ranges_follow_the_same_rule() {
        assert_eq!(Capability::<u32>::from_option_range(None), Capability::Unknown);
        assert_eq!(Capability::from_option_range(Some((5u32, 1))), Capability::Unsupported, "an inverted range offers nothing");
        let r = Capability::from_option_range(Some((1u32, 5)));
        assert_eq!(r.choices(), Some(&Choices::Range { min: 1, max: 5 }));
        assert!(r.list().is_empty(), "a range has no list");
        assert!(Capability::from_option_range(Some((3u32, 3))).is_supported(), "a one-value range is a range");
    }

    #[test]
    fn the_device_default_goes_first_and_must_be_offered() {
        let c = Capability::list_default_first(&["a", "b", "c"], Some(&"b"));
        assert_eq!(c.list(), ["b", "a", "c"]);
        assert_eq!(c.default_value(), Some(&"b"));
        let c = Capability::list_default_first(&["a", "b"], Some(&"z"));
        assert_eq!((c.list(), c.default_value()), (&["a", "b"][..], None), "an unlisted default is ignored");
        assert_eq!(Capability::list_default_first(&["a", "b"], None).list(), ["a", "b"]);
        assert_eq!(Capability::list_default_first(&["a", "a", "b"], Some(&"a")).list(), ["a", "b"], "duplicates of the default collapse into it");
        assert_eq!(Capability::<&str>::list_default_first(&[], Some(&"a")), Capability::Unsupported);
        let d = Capability::from_option_list(Some(&[1u32, 2, 3][..])).with_default(2);
        assert_eq!((d.list(), d.default_value()), (&[1, 2, 3][..], Some(&2)), "with_default keeps the order");
        assert_eq!(Capability::from_option_list(Some(&[1u32][..])).with_default(9).default_value(), None);
        assert_eq!(Capability::from_option_range(Some((1u32, 5))).with_default(4).default_value(), Some(&4));
        assert_eq!(Capability::<u32>::Unknown.with_default(1), Capability::Unknown);
        assert_eq!(Capability::<u32>::Unsupported.with_default(1), Capability::Unsupported);
    }

    #[test]
    fn the_three_states_answer_the_accessors_differently() {
        let unknown = Capability::<u32>::Unknown;
        let unsupported = Capability::<u32>::Unsupported;
        let supported = Capability::from_option_list(Some(&[1u32, 2][..]));
        assert_eq!([unknown.is_known(), unsupported.is_known(), supported.is_known()], [false, true, true]);
        assert_eq!([unknown.is_enabled(), unsupported.is_enabled(), supported.is_enabled()], [true, false, true]);
        assert_eq!([unknown.is_supported(), unsupported.is_supported(), supported.is_supported()], [false, false, true]);
        assert_eq!([unknown.is_unsupported(), unsupported.is_unsupported(), supported.is_unsupported()], [false, true, false]);
        assert!(unknown.choices().is_none() && unsupported.choices().is_none() && supported.choices().is_some());
        assert!(unknown.list().is_empty() && unsupported.list().is_empty());
    }

    #[test]
    fn allows_and_clamp_for_lists_ranges_and_the_other_states() {
        let list = Capability::from_option_list(Some(&[1u32, 3][..]));
        assert!(list.allows(&3) && !list.allows(&2));
        assert_eq!((list.clamp_to_supported(&3), list.clamp_to_supported(&2)), (Some(3), None), "a value not listed is dropped, not invented");
        let range = Capability::from_option_range(Some((2u32, 5)));
        assert!(range.allows(&2) && range.allows(&5) && !range.allows(&1) && !range.allows(&6));
        assert_eq!([0u32, 2, 4, 5, 9].map(|v| range.clamp_to_supported(&v)), [Some(2), Some(2), Some(4), Some(5), Some(5)]);
        assert!(Capability::<u32>::Unknown.allows(&7), "an unknown option takes a hint");
        assert_eq!(Capability::<u32>::Unknown.clamp_to_supported(&7), Some(7));
        assert!(!Capability::<u32>::Unsupported.allows(&7));
        assert_eq!(Capability::<u32>::Unsupported.clamp_to_supported(&7), None);
        let floats = Capability::from_option_range(Some((0.5f32, 1.5)));
        assert_eq!(floats.clamp_to_supported(&2.0), Some(1.5), "partial order is enough");
    }
}
