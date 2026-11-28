#![allow(unsafe_code)]

#[allow(unsafe_code)]
const REGISTRATION: u8 = 0;

#[expect(unsafe_code, reason = "link section")]
const OTHER: u8 = 0;
