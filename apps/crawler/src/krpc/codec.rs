use bytes::{BufMut, Bytes, BytesMut};
use std::collections::BTreeMap;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BValue {
    Int(i64),
    Bytes(Bytes),
    List(Vec<BValue>),
    Dict(BTreeMap<Bytes, BValue>),
}

impl BValue {
    pub fn as_int(&self) -> Option<i64> {
        match self {
            BValue::Int(i) => Some(*i),
            _ => None,
        }
    }

    pub fn as_bytes(&self) -> Option<&Bytes> {
        match self {
            BValue::Bytes(b) => Some(b),
            _ => None,
        }
    }

    pub fn as_dict(&self) -> Option<&BTreeMap<Bytes, BValue>> {
        match self {
            BValue::Dict(d) => Some(d),
            _ => None,
        }
    }

    pub fn as_list(&self) -> Option<&[BValue]> {
        match self {
            BValue::List(l) => Some(l),
            _ => None,
        }
    }

    pub fn get(&self, key: &[u8]) -> Option<&BValue> {
        self.as_dict().and_then(|d| d.get(key))
    }

    pub fn get_bytes(&self, key: &[u8]) -> Option<&Bytes> {
        self.get(key).and_then(BValue::as_bytes)
    }

    pub fn get_int(&self, key: &[u8]) -> Option<i64> {
        self.get(key).and_then(BValue::as_int)
    }

    pub fn dict(mut entries: Vec<(Bytes, BValue)>) -> BValue {
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        BValue::Dict(entries.into_iter().collect())
    }
}

#[derive(Debug)]
pub struct DecodeError {
    pub msg: &'static str,
    pub pos: usize,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at offset {}", self.msg, self.pos)
    }
}

impl std::error::Error for DecodeError {}

pub fn decode(input: &Bytes) -> Result<BValue, DecodeError> {
    let mut p = Parser {
        input,
        pos: 0,
        depth: 0,
    };
    let v = p.value()?;
    if p.pos != input.len() {
        return Err(DecodeError {
            msg: "trailing data",
            pos: p.pos,
        });
    }
    Ok(v)
}

#[cfg(test)]
pub fn decode_slice(input: &[u8]) -> Result<BValue, DecodeError> {
    decode(&Bytes::copy_from_slice(input))
}

pub fn decode_prefix(input: &Bytes) -> Result<(BValue, usize), DecodeError> {
    let mut p = Parser {
        input,
        pos: 0,
        depth: 0,
    };
    let v = p.value()?;
    Ok((v, p.pos))
}

struct Parser<'a> {
    input: &'a Bytes,
    pos: usize,
    depth: usize,
}

const MAX_DEPTH: usize = 64;

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }

    fn value(&mut self) -> Result<BValue, DecodeError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(DecodeError {
                msg: "nesting too deep",
                pos: self.pos,
            });
        }
        let r = match self.peek() {
            Some(b'i') => self.parse_int(),
            Some(b'l') => self.parse_list(),
            Some(b'd') => self.parse_dict(),
            Some(b'0'..=b'9') => self.parse_bytes(),
            _ => Err(DecodeError {
                msg: "invalid bencode token",
                pos: self.pos,
            }),
        };
        self.depth -= 1;
        r
    }

    fn parse_int(&mut self) -> Result<BValue, DecodeError> {
        let start = self.pos;
        self.pos += 1;
        let mut negative = false;
        if self.peek() == Some(b'-') {
            negative = true;
            self.pos += 1;
        }
        let num_start = self.pos;
        let mut val: i64 = 0;
        let mut digits = 0;
        while let Some(c) = self.peek() {
            if c == b'e' {
                if digits == 0 {
                    return Err(DecodeError {
                        msg: "empty integer",
                        pos: start,
                    });
                }
                self.pos += 1;
                let v = if negative { -val } else { val };
                return Ok(BValue::Int(v));
            }
            if !c.is_ascii_digit() {
                return Err(DecodeError {
                    msg: "invalid integer digit",
                    pos: self.pos,
                });
            }
            val = val
                .checked_mul(10)
                .and_then(|v| v.checked_add((c - b'0') as i64))
                .ok_or(DecodeError {
                    msg: "integer overflow",
                    pos: start,
                })?;
            digits += 1;
            self.pos += 1;
        }
        let _ = num_start;
        Err(DecodeError {
            msg: "unterminated integer",
            pos: start,
        })
    }

    fn parse_bytes(&mut self) -> Result<BValue, DecodeError> {
        let start = self.pos;
        let mut len: usize = 0;
        let mut digits = 0;
        while let Some(c) = self.peek() {
            if c == b':' {
                self.pos += 1;
                if digits == 0 {
                    return Err(DecodeError {
                        msg: "empty length",
                        pos: start,
                    });
                }
                let end = self.pos.checked_add(len).ok_or(DecodeError {
                    msg: "length overflow",
                    pos: start,
                })?;
                if end > self.input.len() {
                    return Err(DecodeError {
                        msg: "string length exceeds buffer",
                        pos: start,
                    });
                }
                let b = self.input.slice(self.pos..end);
                self.pos = end;
                return Ok(BValue::Bytes(b));
            }
            if !c.is_ascii_digit() {
                return Err(DecodeError {
                    msg: "invalid length digit",
                    pos: self.pos,
                });
            }
            len = len
                .checked_mul(10)
                .and_then(|l| l.checked_add((c - b'0') as usize))
                .ok_or(DecodeError {
                    msg: "length overflow",
                    pos: start,
                })?;
            digits += 1;
            self.pos += 1;
        }
        Err(DecodeError {
            msg: "unterminated string",
            pos: start,
        })
    }

    fn parse_list(&mut self) -> Result<BValue, DecodeError> {
        let start = self.pos;
        self.pos += 1;
        let mut items = Vec::new();
        loop {
            match self.peek() {
                Some(b'e') => {
                    self.pos += 1;
                    return Ok(BValue::List(items));
                }
                Some(_) => items.push(self.value()?),
                None => {
                    return Err(DecodeError {
                        msg: "unterminated list",
                        pos: start,
                    });
                }
            }
        }
    }

    fn parse_dict(&mut self) -> Result<BValue, DecodeError> {
        let start = self.pos;
        self.pos += 1;
        let mut entries = BTreeMap::new();
        loop {
            match self.peek() {
                Some(b'e') => {
                    self.pos += 1;
                    return Ok(BValue::Dict(entries));
                }
                Some(b'0'..=b'9') => {
                    let key = self.parse_bytes()?;
                    let key = match key {
                        BValue::Bytes(b) => b,
                        _ => unreachable!(),
                    };
                    let val = self.value()?;
                    entries.insert(key, val);
                }
                Some(_) => {
                    return Err(DecodeError {
                        msg: "dict key must be a string",
                        pos: self.pos,
                    });
                }
                None => {
                    return Err(DecodeError {
                        msg: "unterminated dict",
                        pos: start,
                    });
                }
            }
        }
    }
}

pub fn encode(v: &BValue, out: &mut BytesMut) {
    match v {
        BValue::Int(i) => {
            out.put_u8(b'i');
            out.put_slice(i.to_string().as_bytes());
            out.put_u8(b'e');
        }
        BValue::Bytes(b) => {
            out.put_slice(b.len().to_string().as_bytes());
            out.put_u8(b':');
            out.put_slice(b);
        }
        BValue::List(items) => {
            out.put_u8(b'l');
            for item in items {
                encode(item, out);
            }
            out.put_u8(b'e');
        }
        BValue::Dict(entries) => {
            out.put_u8(b'd');
            for (k, val) in entries {
                encode(&BValue::Bytes(k.clone()), out);
                encode(val, out);
            }
            out.put_u8(b'e');
        }
    }
}

pub fn encode_to_bytes(v: &BValue) -> Bytes {
    let mut out = BytesMut::with_capacity(256);
    encode(v, &mut out);
    out.freeze()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dec(s: &[u8]) -> BValue {
        decode_slice(s).unwrap()
    }

    #[test]
    fn roundtrip() {
        let v = BValue::dict(vec![
            (
                Bytes::from_static(b"t"),
                BValue::Bytes(Bytes::from_static(b"aa")),
            ),
            (
                Bytes::from_static(b"y"),
                BValue::Bytes(Bytes::from_static(b"q")),
            ),
            (
                Bytes::from_static(b"a"),
                BValue::Dict(BTreeMap::from([
                    (
                        Bytes::from_static(b"id"),
                        BValue::Bytes(Bytes::from_static(b"12345678901234567890")),
                    ),
                    (Bytes::from_static(b"port"), BValue::Int(6881)),
                ])),
            ),
        ]);
        let enc = encode_to_bytes(&v);
        let back = decode(&enc).unwrap();
        assert_eq!(v, back);
    }

    #[test]
    fn ints() {
        assert_eq!(dec(b"i0e"), BValue::Int(0));
        assert_eq!(dec(b"i-42e"), BValue::Int(-42));
        assert_eq!(dec(b"i9223372036854775807e"), BValue::Int(i64::MAX));
        assert!(decode_slice(b"i-9223372036854775809e").is_err());
        assert!(decode_slice(b"ie").is_err());
        assert!(decode_slice(b"i").is_err());
    }

    #[test]
    fn strings() {
        assert_eq!(dec(b"4:spam"), BValue::Bytes(Bytes::from_static(b"spam")));
        assert_eq!(dec(b"0:"), BValue::Bytes(Bytes::new()));
        assert!(decode_slice(b"4:spa").is_err());
        assert!(decode_slice(b"-1:x").is_err());
    }

    #[test]
    fn lists_and_dicts() {
        assert_eq!(
            dec(b"l4:spami42ee"),
            BValue::List(vec![
                BValue::Bytes(Bytes::from_static(b"spam")),
                BValue::Int(42),
            ])
        );
        let d = dec(b"d3:bar4:spam3:fooi42ee");
        match d {
            BValue::Dict(m) => {
                assert_eq!(m.len(), 2);
                assert_eq!(m.get(b"foo".as_slice()), Some(&BValue::Int(42)));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn fuzz_malformed() {
        let seeds: &[&[u8]] = &[
            b"",
            b"i",
            b"l",
            b"d",
            b"d3:fooi42",
            b"i-0e",
            b"01:x",
            b"d3:foo",
            b"li42ee",
            b"d0:0:0:0:0:0:0:0:0:0:0:0:0:0:0:0:0:0:0:0:0:0:0:0:0:0:e",
            b"\xff\xff\xff",
            b"4:abce",
            b"i1ei2e",
        ];
        for seed in seeds {
            let _ = decode_slice(seed);
        }
    }
}
