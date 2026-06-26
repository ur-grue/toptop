//! Minimal, dependency-free JSON parser — enough to read back the documents
//! produced by `toptop --export json` for the multi-host fleet view.
//!
//! It's a small recursive-descent parser over a borrowed string. The goal is
//! correctness on well-formed input (including escapes and UTF-8), not a
//! complete validator; malformed input returns `None` rather than panicking.

/// A parsed JSON value.
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    /// Look up a key in an object.
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Num(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_u64(&self) -> Option<u64> {
        self.as_f64().map(|n| n.max(0.0) as u64)
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Json]> {
        match self {
            Json::Arr(a) => Some(a),
            _ => None,
        }
    }

    /// Convenience: `obj.get(key).and_then(as_f64)`.
    pub fn num(&self, key: &str) -> Option<f64> {
        self.get(key).and_then(Json::as_f64)
    }

    /// Convenience: `obj.get(key).and_then(as_u64)`.
    pub fn u64(&self, key: &str) -> Option<u64> {
        self.get(key).and_then(Json::as_u64)
    }

    /// Convenience: `obj.get(key).and_then(as_str)`.
    pub fn str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Json::as_str)
    }
}

/// Parse a JSON document. Returns `None` on malformed input.
pub fn parse(input: &str) -> Option<Json> {
    let mut p = Parser {
        b: input.as_bytes(),
        i: 0,
    };
    p.ws();
    let v = p.value()?;
    p.ws();
    Some(v)
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

fn utf8_len(b: u8) -> usize {
    if b < 0x80 {
        1
    } else if b >> 5 == 0b110 {
        2
    } else if b >> 4 == 0b1110 {
        3
    } else if b >> 3 == 0b11110 {
        4
    } else {
        1
    }
}

impl Parser<'_> {
    fn ws(&mut self) {
        while matches!(self.b.get(self.i), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.i += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    fn value(&mut self) -> Option<Json> {
        self.ws();
        match self.peek()? {
            b'{' => self.object(),
            b'[' => self.array(),
            b'"' => self.string().map(Json::Str),
            b't' | b'f' => self.boolean(),
            b'n' => self.literal("null", Json::Null),
            _ => self.number(),
        }
    }

    fn object(&mut self) -> Option<Json> {
        self.i += 1; // consume '{'
        let mut out = Vec::new();
        self.ws();
        if self.peek() == Some(b'}') {
            self.i += 1;
            return Some(Json::Obj(out));
        }
        loop {
            self.ws();
            let k = self.string()?;
            self.ws();
            if self.peek() != Some(b':') {
                return None;
            }
            self.i += 1;
            let v = self.value()?;
            out.push((k, v));
            self.ws();
            match self.peek()? {
                b',' => self.i += 1,
                b'}' => {
                    self.i += 1;
                    break;
                }
                _ => return None,
            }
        }
        Some(Json::Obj(out))
    }

    fn array(&mut self) -> Option<Json> {
        self.i += 1; // consume '['
        let mut out = Vec::new();
        self.ws();
        if self.peek() == Some(b']') {
            self.i += 1;
            return Some(Json::Arr(out));
        }
        loop {
            let v = self.value()?;
            out.push(v);
            self.ws();
            match self.peek()? {
                b',' => self.i += 1,
                b']' => {
                    self.i += 1;
                    break;
                }
                _ => return None,
            }
        }
        Some(Json::Arr(out))
    }

    fn string(&mut self) -> Option<String> {
        if self.peek() != Some(b'"') {
            return None;
        }
        self.i += 1;
        let mut s = String::new();
        loop {
            let c = *self.b.get(self.i)?;
            if c == b'"' {
                self.i += 1;
                break;
            }
            if c == b'\\' {
                self.i += 1;
                let e = *self.b.get(self.i)?;
                self.i += 1;
                match e {
                    b'"' => s.push('"'),
                    b'\\' => s.push('\\'),
                    b'/' => s.push('/'),
                    b'n' => s.push('\n'),
                    b't' => s.push('\t'),
                    b'r' => s.push('\r'),
                    b'b' => s.push('\u{8}'),
                    b'f' => s.push('\u{c}'),
                    b'u' => {
                        if self.i + 4 > self.b.len() {
                            return None;
                        }
                        let hex = std::str::from_utf8(&self.b[self.i..self.i + 4]).ok()?;
                        let cp = u32::from_str_radix(hex, 16).ok()?;
                        self.i += 4;
                        s.push(char::from_u32(cp).unwrap_or('\u{fffd}'));
                    }
                    _ => return None,
                }
            } else {
                let len = utf8_len(c);
                let end = self.i + len;
                let chunk = self.b.get(self.i..end)?;
                let ch = std::str::from_utf8(chunk).ok()?.chars().next()?;
                s.push(ch);
                self.i = end;
            }
        }
        Some(s)
    }

    fn number(&mut self) -> Option<Json> {
        let start = self.i;
        while let Some(c) = self.peek() {
            if matches!(c, b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E') {
                self.i += 1;
            } else {
                break;
            }
        }
        let s = std::str::from_utf8(&self.b[start..self.i]).ok()?;
        s.parse::<f64>().ok().map(Json::Num)
    }

    fn boolean(&mut self) -> Option<Json> {
        if self.b[self.i..].starts_with(b"true") {
            self.i += 4;
            Some(Json::Bool(true))
        } else if self.b[self.i..].starts_with(b"false") {
            self.i += 5;
            Some(Json::Bool(false))
        } else {
            None
        }
    }

    fn literal(&mut self, word: &str, val: Json) -> Option<Json> {
        if self.b[self.i..].starts_with(word.as_bytes()) {
            self.i += word.len();
            Some(val)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primitives() {
        assert_eq!(parse("null"), Some(Json::Null));
        assert_eq!(parse("true"), Some(Json::Bool(true)));
        assert_eq!(parse(" 42 "), Some(Json::Num(42.0)));
        assert_eq!(parse("-1.5e2"), Some(Json::Num(-150.0)));
        assert_eq!(parse(r#""hi\n\"x\"""#), Some(Json::Str("hi\n\"x\"".into())));
    }

    #[test]
    fn nested() {
        let v = parse(r#"{"a":[1,2,{"b":"c"}],"d":null,"e":true}"#).unwrap();
        assert_eq!(v.get("a").unwrap().as_array().unwrap().len(), 3);
        assert_eq!(
            v.get("a").unwrap().as_array().unwrap()[1].as_f64(),
            Some(2.0)
        );
        assert_eq!(
            v.get("a").unwrap().as_array().unwrap()[2].str("b"),
            Some("c")
        );
        assert_eq!(v.get("d"), Some(&Json::Null));
        assert_eq!(v.get("e").unwrap(), &Json::Bool(true));
    }

    #[test]
    fn unicode_and_escapes() {
        let v = parse(r#"{"s":"café ❤"}"#).unwrap();
        assert_eq!(v.str("s"), Some("café ❤"));
        // Raw UTF-8 bytes in the input are preserved too.
        assert_eq!(parse(r#""náïve""#).unwrap().as_str(), Some("náïve"));
    }

    #[test]
    fn malformed_is_none() {
        assert_eq!(parse("{"), None);
        assert_eq!(parse("[1,]"), None);
        assert_eq!(parse(""), None);
    }
}
