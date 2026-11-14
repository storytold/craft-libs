//! The child program of the `bin` tests. The first argument picks what it does:
//!
//! - `out <n>` and `err <n>` write `n` bytes to stdout or stderr and exit.
//! - `out-then-wait <n>` and `err-then-wait <n>` write `n` bytes and then wait forever.
//! - `err-then-out <n>` writes `n` bytes to stderr, then `n` bytes to stdout.
//! - `stdin` reads stdin to its end and writes the number of bytes it read to stdout.
//! - `env <name>` writes the value of the variable `name` to stdout, or exits with 3 when it is
//!   not set.
//! - `exit <code>` writes `bye` to stderr and exits with `code`.

use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::process::ExitCode;

fn main() -> ExitCode {
    match child() {
        Ok(code) => code,
        Err(error) => {
            let _ = writeln!(io::stderr(), "craft-testkit-fixture: {error}");
            ExitCode::from(101)
        }
    }
}

fn child() -> io::Result<ExitCode> {
    let mut args = std::env::args_os().skip(1);
    let mode = args.next().and_then(|mode| mode.into_string().ok()).unwrap_or_default();
    let operand = args.next();
    match mode.as_str() {
        "out" => write_bytes(&mut io::stdout(), count(operand)?).map(|()| ExitCode::SUCCESS),
        "err" => write_bytes(&mut io::stderr(), count(operand)?).map(|()| ExitCode::SUCCESS),
        "out-then-wait" => {
            write_bytes(&mut io::stdout(), count(operand)?)?;
            wait_forever()
        }
        "err-then-wait" => {
            write_bytes(&mut io::stderr(), count(operand)?)?;
            wait_forever()
        }
        "err-then-out" => {
            let n = count(operand)?;
            write_bytes(&mut io::stderr(), n)?;
            write_bytes(&mut io::stdout(), n)?;
            Ok(ExitCode::SUCCESS)
        }
        "stdin" => {
            let mut input = Vec::new();
            io::stdin().read_to_end(&mut input)?;
            write!(io::stdout(), "{}", input.len())?;
            Ok(ExitCode::SUCCESS)
        }
        "env" => {
            let Some(value) = operand.and_then(std::env::var_os) else {
                return Ok(ExitCode::from(3));
            };
            io::stdout().write_all(&value_bytes(value))?;
            Ok(ExitCode::SUCCESS)
        }
        "exit" => {
            let code = operand.and_then(|code| code.into_string().ok()).and_then(|code| code.parse::<u8>().ok()).unwrap_or(1);
            io::stderr().write_all(b"bye")?;
            Ok(ExitCode::from(code))
        }
        _ => Err(io::Error::new(io::ErrorKind::InvalidInput, format!("unknown mode {mode:?}"))),
    }
}

fn count(operand: Option<OsString>) -> io::Result<usize> {
    operand
        .and_then(|n| n.into_string().ok())
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "expected a byte count"))
}

fn write_bytes(stream: &mut impl Write, n: usize) -> io::Result<()> {
    let chunk = [b'x'; 8192];
    let mut left = n;
    while left > 0 {
        let len = left.min(chunk.len());
        stream.write_all(&chunk[..len])?;
        left -= len;
    }
    stream.flush()
}

fn wait_forever() -> ! {
    loop {
        std::thread::park();
    }
}

#[cfg(unix)]
fn value_bytes(value: OsString) -> Vec<u8> {
    use std::os::unix::ffi::OsStringExt;
    value.into_vec()
}

#[cfg(not(unix))]
fn value_bytes(value: OsString) -> Vec<u8> {
    value.to_string_lossy().into_owned().into_bytes()
}
