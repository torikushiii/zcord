use serde_json::{Map, Number, Value};

const VERSION: u8 = 131;
const NEW_FLOAT: u8 = 70;
const SMALL_INTEGER: u8 = 97;
const INTEGER: u8 = 98;
const ATOM: u8 = 100;
const SMALL_TUPLE: u8 = 104;
const LARGE_TUPLE: u8 = 105;
const NIL: u8 = 106;
const STRING: u8 = 107;
const LIST: u8 = 108;
const BINARY: u8 = 109;
const SMALL_BIG: u8 = 110;
const LARGE_BIG: u8 = 111;
const SMALL_ATOM: u8 = 115;
const MAP: u8 = 116;
const ATOM_UTF8: u8 = 118;
const SMALL_ATOM_UTF8: u8 = 119;

const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;
const MAX_DEPTH: usize = 128;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct DecodeError;

pub(super) fn decode(bytes: &[u8]) -> Result<Value, DecodeError> {
    let mut reader = Reader { bytes, at: 0 };
    if reader.u8()? != VERSION {
        return Err(DecodeError);
    }
    let value = reader.term(0)?;
    if reader.at != bytes.len() {
        return Err(DecodeError);
    }
    Ok(value)
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8], DecodeError> {
        let end = self.at.checked_add(len).ok_or(DecodeError)?;
        let slice = self.bytes.get(self.at..end).ok_or(DecodeError)?;
        self.at = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<usize, DecodeError> {
        let bytes = self.take(2)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]).into())
    }

    fn u32(&mut self) -> Result<usize, DecodeError> {
        let bytes = self.take(4)?;
        usize::try_from(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
            .map_err(|_| DecodeError)
    }

    fn remaining(&self) -> usize {
        self.bytes.len() - self.at
    }

    fn term(&mut self, depth: usize) -> Result<Value, DecodeError> {
        if depth > MAX_DEPTH {
            return Err(DecodeError);
        }
        Ok(match self.u8()? {
            NEW_FLOAT => {
                let bytes = self.take(8)?.try_into().map_err(|_| DecodeError)?;
                Number::from_f64(f64::from_be_bytes(bytes))
                    .map(Value::Number)
                    .ok_or(DecodeError)?
            }
            SMALL_INTEGER => self.u8()?.into(),
            INTEGER => {
                let bytes = self.take(4)?.try_into().map_err(|_| DecodeError)?;
                i32::from_be_bytes(bytes).into()
            }
            ATOM | ATOM_UTF8 => {
                let len = self.u16()?;
                atom(self.take(len)?)
            }
            SMALL_ATOM | SMALL_ATOM_UTF8 => {
                let len = self.u8()?.into();
                atom(self.take(len)?)
            }
            SMALL_TUPLE => {
                let arity = self.u8()?.into();
                Value::Array(self.terms(arity, depth)?)
            }
            LARGE_TUPLE => {
                let arity = self.u32()?;
                Value::Array(self.terms(arity, depth)?)
            }
            NIL => Value::Array(Vec::new()),
            STRING => {
                let len = self.u16()?;
                Value::Array(self.take(len)?.iter().map(|&byte| byte.into()).collect())
            }
            LIST => {
                let len = self.u32()?;
                let mut items = self.terms(len, depth)?;
                match self.term(depth + 1)? {
                    Value::Array(tail) if tail.is_empty() => {}
                    tail => items.push(tail),
                }
                Value::Array(items)
            }
            BINARY => {
                let len = self.u32()?;
                Value::String(String::from_utf8_lossy(self.take(len)?).into_owned())
            }
            SMALL_BIG => {
                let len = self.u8()?.into();
                big(self.u8()?, self.take(len)?)?
            }
            LARGE_BIG => {
                let len = self.u32()?;
                big(self.u8()?, self.take(len)?)?
            }
            MAP => {
                let arity = self.u32()?;
                let mut map = Map::with_capacity(arity.min(self.remaining() / 2));
                for _ in 0..arity {
                    let key = match self.term(depth + 1)? {
                        Value::String(key) => key,
                        Value::Number(key) => key.to_string(),
                        Value::Bool(key) => key.to_string(),
                        Value::Null => "null".to_owned(),
                        Value::Array(_) | Value::Object(_) => return Err(DecodeError),
                    };
                    map.insert(key, self.term(depth + 1)?);
                }
                Value::Object(map)
            }
            _ => return Err(DecodeError),
        })
    }

    fn terms(&mut self, count: usize, depth: usize) -> Result<Vec<Value>, DecodeError> {
        let mut items = Vec::with_capacity(count.min(self.remaining()));
        for _ in 0..count {
            items.push(self.term(depth + 1)?);
        }
        Ok(items)
    }
}

fn atom(name: &[u8]) -> Value {
    match name {
        b"nil" | b"null" => Value::Null,
        b"true" => Value::Bool(true),
        b"false" => Value::Bool(false),
        name => Value::String(String::from_utf8_lossy(name).into_owned()),
    }
}

/// Snowflakes can arrive as integers, so anything past 2^53 must stay a decimal string: that is
/// the shape the JSON encoding gives them and the shape the models parse.
/// https://docs.discord.food/gateway/using-gateway#using-etf-encoding
fn big(sign: u8, digits: &[u8]) -> Result<Value, DecodeError> {
    let significant = digits
        .iter()
        .rposition(|&byte| byte != 0)
        .map_or(0, |at| at + 1);
    if significant > 16 || sign > 1 {
        return Err(DecodeError);
    }
    let magnitude = digits[..significant]
        .iter()
        .rev()
        .fold(0u128, |value, &byte| value << 8 | u128::from(byte));
    let negative = sign == 1 && magnitude != 0;
    if magnitude <= u128::from(MAX_SAFE_INTEGER) {
        let magnitude = magnitude as i64;
        return Ok(if negative { -magnitude } else { magnitude }.into());
    }
    Ok(Value::String(if negative {
        format!("-{magnitude}")
    } else {
        magnitude.to_string()
    }))
}

pub(super) fn encode(value: &Value) -> Vec<u8> {
    let mut out = vec![VERSION];
    write(&mut out, value);
    out
}

fn write(out: &mut Vec<u8>, value: &Value) {
    match value {
        Value::Null => write_atom(out, "nil"),
        Value::Bool(true) => write_atom(out, "true"),
        Value::Bool(false) => write_atom(out, "false"),
        Value::Number(number) => write_number(out, number),
        Value::String(text) => write_binary(out, text),
        Value::Array(items) if items.is_empty() => out.push(NIL),
        Value::Array(items) => {
            out.push(LIST);
            write_len(out, items.len());
            for item in items {
                write(out, item);
            }
            out.push(NIL);
        }
        Value::Object(map) => {
            out.push(MAP);
            write_len(out, map.len());
            for (key, value) in map {
                write_binary(out, key);
                write(out, value);
            }
        }
    }
}

fn write_len(out: &mut Vec<u8>, len: usize) {
    let len = u32::try_from(len).expect("gateway payloads stay far below 4 GiB");
    out.extend(len.to_be_bytes());
}

fn write_atom(out: &mut Vec<u8>, name: &str) {
    out.extend([SMALL_ATOM, name.len() as u8]);
    out.extend(name.as_bytes());
}

fn write_binary(out: &mut Vec<u8>, text: &str) {
    out.push(BINARY);
    write_len(out, text.len());
    out.extend(text.as_bytes());
}

fn write_number(out: &mut Vec<u8>, number: &Number) {
    let integer = if let Some(value) = number.as_u64() {
        Some((false, value))
    } else if let Some(value) = number.as_i64() {
        Some((true, value.unsigned_abs()))
    } else {
        number
            .as_f64()
            .filter(|value| value.fract() == 0.0 && value.abs() <= MAX_SAFE_INTEGER as f64)
            .map(|value| (value < 0.0, value.abs() as u64))
    };
    let Some((negative, magnitude)) = integer else {
        out.push(NEW_FLOAT);
        out.extend(number.as_f64().unwrap_or_default().to_be_bytes());
        return;
    };
    if !negative && magnitude <= u64::from(u8::MAX) {
        out.extend([SMALL_INTEGER, magnitude as u8]);
    } else if magnitude < 1 << 31 {
        let value = magnitude as i32;
        out.push(INTEGER);
        out.extend(if negative { -value } else { value }.to_be_bytes());
    } else {
        let digits = magnitude.to_le_bytes();
        let len = digits
            .iter()
            .rposition(|&byte| byte != 0)
            .map_or(0, |at| at + 1);
        out.extend([SMALL_BIG, len as u8, u8::from(negative)]);
        out.extend(&digits[..len]);
    }
}

#[cfg(test)]
mod tests;
