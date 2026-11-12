//! Canonical S-expressions (Rivest's draft, the form gpg-agent uses): lists in parentheses and
//! atoms written as their length, a colon and their bytes (`(7:sig-val(3:rsa(1:s3:abc)))`).
//!
//! The reader takes the canonical form only, and holds what it reads to caps: [`MAX_INPUT`]
//! bytes, [`MAX_DEPTH`] lists deep and [`MAX_NODES`] atoms and lists. A display hint
//! (`[4:text]` before an atom) is read past. Atoms are wiped from memory when they are dropped.

/// Longest S-expression read.
pub const MAX_INPUT: usize = 256 * 1024;
/// Deepest nesting of lists.
pub const MAX_DEPTH: usize = 32;
/// Most atoms and lists in one S-expression.
pub const MAX_NODES: usize = 4096;
/// Most digits of an atom's length.
const MAX_LENGTH_DIGITS: usize = 9;

/// An S-expression.
#[derive(Clone, PartialEq, Eq)]
pub enum Sexp {
    Atom(Vec<u8>),
    List(Vec<Sexp>),
}

impl Drop for Sexp {
    fn drop(&mut self) {
        if let Sexp::Atom(bytes) = self {
            crate::wipe(bytes);
        }
    }
}

impl std::fmt::Debug for Sexp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // A token is shown; any other atom may be secret, and only its size is.
            Sexp::Atom(b) if is_token(b) => write!(f, "{}", String::from_utf8_lossy(b)),
            Sexp::Atom(b) => write!(f, "<{} bytes>", b.len()),
            Sexp::List(items) => f.debug_list().entries(items).finish(),
        }
    }
}

fn is_token(bytes: &[u8]) -> bool {
    !bytes.is_empty()
        && bytes.len() <= 32
        && bytes.first().is_some_and(|b| b.is_ascii_alphabetic())
        && bytes.iter().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// Why an S-expression couldn't be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SexpError {
    #[error("the S-expression ends early")]
    Truncated,
    #[error("the S-expression is too big")]
    TooBig,
    #[error("the S-expression is nested too deeply")]
    TooDeep,
    #[error("the S-expression isn't in canonical form at byte {0}")]
    Malformed(usize),
    #[error("something follows the S-expression")]
    Trailing,
}

/// Read one canonical S-expression that is the whole of `input`.
pub fn parse(input: &[u8]) -> Result<Sexp, SexpError> {
    if input.len() > MAX_INPUT {
        return Err(SexpError::TooBig);
    }
    let mut reader = Reader { input, pos: 0, nodes: 0 };
    let sexp = reader.value(0)?;
    if reader.pos != input.len() {
        return Err(SexpError::Trailing);
    }
    Ok(sexp)
}

struct Reader<'a> {
    input: &'a [u8],
    pos: usize,
    nodes: usize,
}

impl Reader<'_> {
    fn peek(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }

    fn count(&mut self) -> Result<(), SexpError> {
        self.nodes = self.nodes.saturating_add(1);
        if self.nodes > MAX_NODES { Err(SexpError::TooBig) } else { Ok(()) }
    }

    fn value(&mut self, depth: usize) -> Result<Sexp, SexpError> {
        match self.peek() {
            None => Err(SexpError::Truncated),
            Some(b'(') => {
                if depth >= MAX_DEPTH {
                    return Err(SexpError::TooDeep);
                }
                self.count()?;
                self.pos = self.pos.saturating_add(1);
                let mut items = Vec::new();
                loop {
                    match self.peek() {
                        None => return Err(SexpError::Truncated),
                        Some(b')') => {
                            self.pos = self.pos.saturating_add(1);
                            return Ok(Sexp::List(items));
                        }
                        Some(_) => items.push(self.value(depth.saturating_add(1))?),
                    }
                }
            }
            Some(b'[') => {
                // A display hint, then the atom it is for.
                self.pos = self.pos.saturating_add(1);
                drop(self.atom()?);
                if self.peek() != Some(b']') {
                    return if self.peek().is_none() { Err(SexpError::Truncated) } else { Err(SexpError::Malformed(self.pos)) };
                }
                self.pos = self.pos.saturating_add(1);
                if self.peek() == Some(b'[') {
                    return Err(SexpError::Malformed(self.pos));
                }
                self.count()?;
                Ok(Sexp::Atom(self.atom()?))
            }
            Some(_) => {
                self.count()?;
                Ok(Sexp::Atom(self.atom()?))
            }
        }
    }

    /// `<length>:<bytes>`.
    fn atom(&mut self) -> Result<Vec<u8>, SexpError> {
        let start = self.pos;
        let mut length = 0usize;
        let mut digits = 0usize;
        loop {
            match self.peek() {
                None => return Err(SexpError::Truncated),
                Some(b':') if digits > 0 => break,
                Some(d @ b'0'..=b'9') => {
                    // No leading zero (but the length 0 itself).
                    if digits == 1 && length == 0 {
                        return Err(SexpError::Malformed(self.pos));
                    }
                    digits = digits.saturating_add(1);
                    if digits > MAX_LENGTH_DIGITS {
                        return Err(SexpError::TooBig);
                    }
                    length = length.saturating_mul(10).saturating_add(usize::from(d - b'0'));
                    self.pos = self.pos.saturating_add(1);
                }
                Some(_) => return Err(SexpError::Malformed(start)),
            }
        }
        // Past the colon.
        self.pos = self.pos.saturating_add(1);
        let end = self.pos.checked_add(length).ok_or(SexpError::TooBig)?;
        let bytes = self.input.get(self.pos..end).ok_or(SexpError::Truncated)?;
        self.pos = end;
        Ok(bytes.to_vec())
    }
}

impl Sexp {
    /// An atom made of a text token.
    pub fn token(text: &str) -> Sexp {
        Sexp::Atom(text.as_bytes().to_vec())
    }

    /// The bytes of an atom.
    pub fn as_atom(&self) -> Option<&[u8]> {
        match self {
            Sexp::Atom(b) => Some(b),
            Sexp::List(_) => None,
        }
    }

    /// The items of a list.
    pub fn as_list(&self) -> Option<&[Sexp]> {
        match self {
            Sexp::List(items) => Some(items),
            Sexp::Atom(_) => None,
        }
    }

    /// A list's name: its first item, when that is an atom of text.
    pub fn name(&self) -> Option<&str> {
        self.as_list()?.first()?.as_atom().and_then(|b| std::str::from_utf8(b).ok())
    }

    /// The first item of this list that is a list named `name`.
    pub fn find(&self, name: &str) -> Option<&Sexp> {
        self.as_list()?.iter().find(|item| item.name() == Some(name))
    }

    /// The atom of `(name value)` among this list's items.
    pub fn value(&self, name: &str) -> Option<&[u8]> {
        self.find(name)?.as_list()?.get(1)?.as_atom()
    }

    /// The canonical form.
    pub fn to_canonical(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.write(&mut out);
        out
    }

    fn write(&self, out: &mut Vec<u8>) {
        match self {
            Sexp::Atom(b) => {
                out.extend_from_slice(b.len().to_string().as_bytes());
                out.push(b':');
                out.extend_from_slice(b);
            }
            Sexp::List(items) => {
                out.push(b'(');
                for item in items {
                    item.write(out);
                }
                out.push(b')');
            }
        }
    }
}
