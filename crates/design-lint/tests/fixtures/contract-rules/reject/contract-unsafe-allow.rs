#![allow(unsafe_code)]

#[allow(unsafe_code)]
static REGISTRATION: u8 = 0;

#[expect(unsafe_code, reason = "link section")]
static OTHER: u8 = 0;
