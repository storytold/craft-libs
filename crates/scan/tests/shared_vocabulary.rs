//! craft-scan's native code, backend names and namespaced ids are craft-job's. These tests pin that the names
//! and wire formats did not change when they moved.

mod common;

use craft_job::{BackendName, DeviceId};
use craft_scan::{Backend, HRESULT_DOMAIN, NativeCode, NativeCodeExt, RegisterError, Registry, ScanError, namespaced_id, split_id};

#[test]
fn the_native_code_is_craft_jobs_with_the_same_wire_values() {
    let from_job: craft_job::NativeCode = NativeCode::hresult(0x8021_0006, Some("WIA_ERROR_BUSY"));
    let back: NativeCode = from_job;
    assert_eq!((back.domain, back.code, back.name), (HRESULT_DOMAIN, 0x8021_0006, Some("WIA_ERROR_BUSY")));
    assert_eq!(HRESULT_DOMAIN, craft_job::HRESULT_DOMAIN);
    assert_eq!(back.as_hresult(), Some(0x8021_0006));
    assert_eq!(NativeCode::new("sane", 6, None).as_hresult(), None, "only the hresult domain has an HRESULT");
    assert_eq!(NativeCode::new(HRESULT_DOMAIN, -1, None).as_hresult(), None, "a negative code does not fit");
}

#[test]
fn namespaced_ids_keep_their_format_and_parse_as_device_ids() {
    assert_eq!(namespaced_id("escl", "http://10.0.0.2/eSCL"), "escl:http://10.0.0.2/eSCL");
    assert_eq!(split_id("sane:pixma:libusb:001:004"), Some(("sane", "pixma:libusb:001:004")), "only the first colon separates");
    assert_eq!(split_id("nocolon"), None);
    let id: DeviceId = namespaced_id("wia", r"{GUID}\0000").parse().expect("a registry id is a device id");
    assert_eq!((id.backend(), id.native()), ("wia", r"{GUID}\0000"));
}

struct Named(&'static str);

impl Backend for Named {
    fn name(&self) -> &'static str {
        self.0
    }
    fn scanner_list(&self) -> Result<Vec<craft_scan::ScannerRef>, ScanError> {
        Ok(Vec::new())
    }
    fn scanner_details(&self, _native_id: &str) -> Result<craft_scan::Scanner, ScanError> {
        Err(ScanError::Unsupported)
    }
    fn scanners(&self) -> Result<Vec<craft_scan::Scanner>, ScanError> {
        Ok(Vec::new())
    }
    fn diagnostic_dump(&self, _filter: &str) -> Result<String, ScanError> {
        Ok(String::new())
    }
    fn acquire(&self, _settings: &craft_scan::ScanSettings) -> Result<craft_scan::Acquired, ScanError> {
        Err(ScanError::Unsupported)
    }
}

#[test]
fn the_registry_accepts_exactly_the_names_craft_job_accepts() {
    for name in ["", "Wia", "a b", "a:b", "w.ia", "\u{e9}", "wia ", "WIA"] {
        assert!(BackendName::new(name).is_err(), "{name:?} is not a craft-job name");
        let registry = Registry::new();
        assert_eq!(registry.register(Box::new(Named(name))), Err(RegisterError::InvalidName { name }), "{name:?}");
    }
    for name in ["wia", "sane", "escl", "a", "x-1", "x_1", "0"] {
        assert!(BackendName::new(name).is_ok(), "{name:?}");
        let registry = Registry::new();
        assert_eq!(registry.register(Box::new(Named(name))), Ok(()), "{name:?}");
    }
}
