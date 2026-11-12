use super::{Descriptor, Value};

impl Descriptor {
    /// The boolean at `key`.
    pub fn get_bool(&self, key: &str) -> Option<bool> {
        match self.get(key)? {
            Value::Boolean(b) => Some(*b),
            _ => None,
        }
    }

    /// The number at `key`, from any numeric value type.
    pub fn get_number(&self, key: &str) -> Option<f64> {
        match self.get(key)? {
            Value::Double(v) => Some(*v),
            Value::UnitFloat { value, .. } => Some(*value),
            Value::Integer(v) => Some(f64::from(*v)),
            Value::LargeInteger(v) => Some(*v as f64),
            _ => None,
        }
    }

    /// The text at `key`.
    pub fn get_text(&self, key: &str) -> Option<String> {
        match self.get(key)? {
            Value::Text(t) => Some(t.to_string_lossy()),
            _ => None,
        }
    }

    /// The enumerated value at `key`.
    pub fn get_enum(&self, key: &str) -> Option<String> {
        match self.get(key)? {
            Value::Enumerated { value, .. } => Some(String::from_utf8_lossy(value.as_bytes()).into_owned()),
            _ => None,
        }
    }

    /// The object at `key`, plain or global.
    pub fn get_object(&self, key: &str) -> Option<&Descriptor> {
        match self.get(key)? {
            Value::Descriptor(o) | Value::GlobalObject(o) => Some(o),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Id, UnicodeString};
    use super::*;

    fn sample() -> Descriptor {
        Descriptor::new("null")
            .with("enab", Value::Boolean(true))
            .with("doub", Value::Double(0.25))
            .with("Opct", Value::UnitFloat { unit: *b"#Prc", value: 75.5 })
            .with("long", Value::Integer(-5))
            .with("comp", Value::LargeInteger(1 << 40))
            .with("Nm  ", Value::Text(UnicodeString::new("Café")))
            .with("Md  ", Value::Enumerated { type_id: Id::new("BlnM"), value: Id::new("Nrml") })
            .with("Objc", Value::Descriptor(Descriptor::new("Clr ").with("Rd  ", Value::Double(1.0))))
            .with("GlbO", Value::GlobalObject(Descriptor::new("Grsc")))
    }

    #[test]
    fn each_lookup_reads_its_own_value_type() {
        let d = sample();
        assert_eq!(d.get_bool("enab"), Some(true));
        assert_eq!(d.get_number("doub"), Some(0.25));
        assert_eq!(d.get_number("Opct"), Some(75.5));
        assert_eq!(d.get_number("long"), Some(-5.0));
        assert_eq!(d.get_number("comp"), Some((1u64 << 40) as f64));
        assert_eq!(d.get_text("Nm  ").as_deref(), Some("Café"));
        assert_eq!(d.get_enum("Md  ").as_deref(), Some("Nrml"));
        assert_eq!(d.get_object("Objc").and_then(|o| o.get_number("Rd  ")), Some(1.0));
        assert!(d.get_object("GlbO").is_some());
    }

    #[test]
    fn lookups_of_other_types_or_missing_keys_are_none() {
        let d = sample();
        assert_eq!(d.get_bool("doub"), None);
        assert_eq!(d.get_number("enab"), None);
        assert_eq!(d.get_text("long"), None);
        assert_eq!(d.get_enum("Nm  "), None);
        assert!(d.get_object("Md  ").is_none());
        for key in ["none", "    "] {
            assert_eq!((d.get_bool(key), d.get_number(key), d.get_text(key), d.get_enum(key)), (None, None, None, None));
            assert!(d.get_object(key).is_none());
        }
    }
}
