//! Binary format used between Rust and the C++ shim (mirror of `shim/obapi_wire.hpp`).
//!
//! | type          | encoding                                  |
//! |---------------|-------------------------------------------|
//! | `bool`        | `u8` 0/1                                  |
//! | integers      | little endian, fixed width                |
//! | `f32`/`f64`   | IEEE-754 little endian                    |
//! | `String`      | `u32` byte length + UTF-8                 |
//! | `Vec<T>`      | `u32` count + elements                    |
//! | `Option<T>`   | like `Vec<T>` with 0 or 1 elements        |
//! | enums         | `u32` ordinal                             |
//! | structs       | fields in declaration order               |

use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum WireError {
    #[error("payload truncated: needed {needed} more bytes")]
    Truncated { needed: usize },

    #[error("{remaining} trailing bytes in payload")]
    Trailing { remaining: usize },

    #[error("invalid bool byte {0}")]
    InvalidBool(u8),

    #[error("invalid UTF-8 in string")]
    InvalidUtf8,

    #[error("optional value with {0} elements")]
    InvalidOptional(u32),

    #[error("invalid value {value} for enum {ty}")]
    InvalidEnum { ty: &'static str, value: u32 },

    #[error("unknown operation id {op} for {interface}")]
    UnknownOp { interface: &'static str, op: u32 },
}

#[derive(Debug, Default, Clone)]
pub struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    pub fn as_bytes(&self) -> &[u8] {
        &self.buf
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.buf
    }

    pub fn put_bytes(&mut self, b: &[u8]) {
        self.buf.extend_from_slice(b);
    }

    pub fn put_len(&mut self, n: usize) {
        // The C++ side rejects longer sequences; 4 billion elements do not occur in practice.
        let n = u32::try_from(n).unwrap_or(u32::MAX);
        self.put_bytes(&n.to_le_bytes());
    }
}

#[derive(Debug, Clone)]
pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    pub fn take(&mut self, n: usize) -> Result<&'a [u8], WireError> {
        let rest = &self.buf[self.pos..];
        if rest.len() < n {
            return Err(WireError::Truncated {
                needed: n - rest.len(),
            });
        }
        self.pos += n;
        Ok(&rest[..n])
    }

    pub fn take_array<const N: usize>(&mut self) -> Result<[u8; N], WireError> {
        let s = self.take(N)?;
        let mut a = [0u8; N];
        a.copy_from_slice(s);
        Ok(a)
    }

    pub fn read_len(&mut self) -> Result<usize, WireError> {
        Ok(u32::from_le_bytes(self.take_array()?) as usize)
    }

    pub fn finish(&self) -> Result<(), WireError> {
        match self.buf.len() - self.pos {
            0 => Ok(()),
            remaining => Err(WireError::Trailing { remaining }),
        }
    }
}

pub trait Encode {
    fn encode(&self, w: &mut Writer);
}

pub trait Decode: Sized {
    fn decode(r: &mut Reader<'_>) -> Result<Self, WireError>;
}

/// Encodes a value into a fresh buffer.
pub fn to_bytes<T: Encode + ?Sized>(v: &T) -> Vec<u8> {
    let mut w = Writer::default();
    v.encode(&mut w);
    w.into_bytes()
}

/// Decodes a value and checks that the whole buffer was consumed.
pub fn from_bytes<T: Decode>(b: &[u8]) -> Result<T, WireError> {
    let mut r = Reader::new(b);
    let v = T::decode(&mut r)?;
    r.finish()?;
    Ok(v)
}

macro_rules! num {
    ($($t:ty),*) => {$(
        impl Encode for $t {
            fn encode(&self, w: &mut Writer) {
                w.put_bytes(&self.to_le_bytes());
            }
        }
        impl Decode for $t {
            fn decode(r: &mut Reader<'_>) -> Result<Self, WireError> {
                Ok(<$t>::from_le_bytes(r.take_array()?))
            }
        }
    )*};
}
num!(u8, i8, u16, i16, u32, i32, u64, i64, f32, f64);

impl Encode for bool {
    fn encode(&self, w: &mut Writer) {
        u8::from(*self).encode(w);
    }
}

impl Decode for bool {
    fn decode(r: &mut Reader<'_>) -> Result<Self, WireError> {
        match u8::decode(r)? {
            0 => Ok(false),
            1 => Ok(true),
            b => Err(WireError::InvalidBool(b)),
        }
    }
}

impl Encode for str {
    fn encode(&self, w: &mut Writer) {
        w.put_len(self.len());
        w.put_bytes(self.as_bytes());
    }
}

impl Encode for String {
    fn encode(&self, w: &mut Writer) {
        self.as_str().encode(w);
    }
}

impl Decode for String {
    fn decode(r: &mut Reader<'_>) -> Result<Self, WireError> {
        let n = r.read_len()?;
        let b = r.take(n)?;
        String::from_utf8(b.to_vec()).map_err(|_| WireError::InvalidUtf8)
    }
}

impl<T: Encode> Encode for [T] {
    fn encode(&self, w: &mut Writer) {
        w.put_len(self.len());
        self.iter().for_each(|x| x.encode(w));
    }
}

impl<T: Encode> Encode for Vec<T> {
    fn encode(&self, w: &mut Writer) {
        self.as_slice().encode(w);
    }
}

impl<T: Decode> Decode for Vec<T> {
    fn decode(r: &mut Reader<'_>) -> Result<Self, WireError> {
        let n = r.read_len()?;
        // Cap the pre-allocation; a corrupt length must not trigger a huge allocation.
        let mut v = Vec::with_capacity(n.min(4096));
        for _ in 0..n {
            v.push(T::decode(r)?);
        }
        Ok(v)
    }
}

impl<T: Encode> Encode for Option<T> {
    fn encode(&self, w: &mut Writer) {
        match self {
            Some(v) => {
                w.put_len(1);
                v.encode(w);
            }
            None => w.put_len(0),
        }
    }
}

impl<T: Decode> Decode for Option<T> {
    fn decode(r: &mut Reader<'_>) -> Result<Self, WireError> {
        match r.read_len()? {
            0 => Ok(None),
            1 => Ok(Some(T::decode(r)?)),
            n => Err(WireError::InvalidOptional(n as u32)),
        }
    }
}

impl<T: Encode + ?Sized> Encode for &T {
    fn encode(&self, w: &mut Writer) {
        (**self).encode(w);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_primitives() {
        let v: (u32, f64, bool, String, Vec<i16>, Option<u8>) =
            (7, 1.5, true, "hä".into(), vec![-1, 2], Some(3));
        let mut w = Writer::default();
        v.0.encode(&mut w);
        v.1.encode(&mut w);
        v.2.encode(&mut w);
        v.3.encode(&mut w);
        v.4.encode(&mut w);
        v.5.encode(&mut w);
        let mut r = Reader::new(w.as_bytes());
        assert_eq!(u32::decode(&mut r).unwrap(), v.0);
        assert_eq!(f64::decode(&mut r).unwrap(), v.1);
        assert_eq!(bool::decode(&mut r).unwrap(), v.2);
        assert_eq!(String::decode(&mut r).unwrap(), v.3);
        assert_eq!(Vec::<i16>::decode(&mut r).unwrap(), v.4);
        assert_eq!(Option::<u8>::decode(&mut r).unwrap(), v.5);
        r.finish().unwrap();
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(from_bytes::<bool>(&[2]), Err(WireError::InvalidBool(2)));
        assert_eq!(
            from_bytes::<u32>(&[1, 2]),
            Err(WireError::Truncated { needed: 2 })
        );
        assert_eq!(
            from_bytes::<Option<u8>>(&[2, 0, 0, 0, 1, 1]),
            Err(WireError::InvalidOptional(2))
        );
        assert_eq!(
            from_bytes::<u8>(&[1, 2]),
            Err(WireError::Trailing { remaining: 1 })
        );
    }
}
