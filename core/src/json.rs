//! Minimal JSON (RFC 8259) parser and writer — enough for model headers,
//! library configuration and metadata, without external dependencies.

use std::fmt::Write as _;

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
    pub fn get(&self, k: &str) -> Option<&Json> {
        match self {
            Json::Obj(v) => v.iter().find(|(a, _)| a == k).map(|(_, b)| b),
            _ => None,
        }
    }
    pub fn str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn num(&self) -> Option<f64> {
        match self {
            Json::Num(n) => Some(*n),
            _ => None,
        }
    }
    pub fn bool(&self) -> Option<bool> {
        match self {
            Json::Bool(b) => Some(*b),
            _ => None,
        }
    }
    pub fn arr(&self) -> Option<&[Json]> {
        match self {
            Json::Arr(a) => Some(a),
            _ => None,
        }
    }
    pub fn obj(&self) -> Option<&[(String, Json)]> {
        match self {
            Json::Obj(o) => Some(o),
            _ => None,
        }
    }
    /// `obj.k` as string, number (formatted) or nothing.
    pub fn get_str(&self, k: &str) -> Option<&str> {
        self.get(k).and_then(|v| v.str())
    }
    pub fn get_num(&self, k: &str) -> Option<f64> {
        self.get(k).and_then(|v| v.num())
    }
    pub fn get_usize(&self, k: &str) -> Option<usize> {
        self.get_num(k).map(|v| v as usize)
    }
    pub fn usizes(&self) -> Vec<usize> {
        self.arr()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.num())
                    .map(|v| v as usize)
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn parse(s: &str) -> Result<Json, String> {
        let mut p = P {
            b: s.as_bytes(),
            i: 0,
        };
        p.ws();
        let v = p.value()?;
        p.ws();
        if p.i != p.b.len() {
            return Err(format!("trailing characters at {}", p.i));
        }
        Ok(v)
    }

    pub fn to_string_compact(&self) -> String {
        let mut o = String::new();
        self.write(&mut o, None, 0);
        o
    }
    pub fn to_string_pretty(&self) -> String {
        let mut o = String::new();
        self.write(&mut o, Some(2), 0);
        o
    }

    fn write(&self, o: &mut String, indent: Option<usize>, lvl: usize) {
        let nl = |o: &mut String, l: usize| {
            if let Some(n) = indent {
                o.push('\n');
                for _ in 0..n * l {
                    o.push(' ');
                }
            }
        };
        match self {
            Json::Null => o.push_str("null"),
            Json::Bool(b) => o.push_str(if *b { "true" } else { "false" }),
            Json::Num(n) => {
                if n.is_finite() && n.fract() == 0.0 && n.abs() < 1e15 {
                    let _ = write!(o, "{}", *n as i64);
                } else if n.is_finite() {
                    let _ = write!(o, "{}", n);
                } else {
                    o.push_str("null");
                }
            }
            Json::Str(s) => write_str(o, s),
            Json::Arr(a) => {
                o.push('[');
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        o.push(',');
                    }
                    nl(o, lvl + 1);
                    v.write(o, indent, lvl + 1);
                }
                if !a.is_empty() {
                    nl(o, lvl);
                }
                o.push(']');
            }
            Json::Obj(m) => {
                o.push('{');
                for (i, (k, v)) in m.iter().enumerate() {
                    if i > 0 {
                        o.push(',');
                    }
                    nl(o, lvl + 1);
                    write_str(o, k);
                    o.push(':');
                    if indent.is_some() {
                        o.push(' ');
                    }
                    v.write(o, indent, lvl + 1);
                }
                if !m.is_empty() {
                    nl(o, lvl);
                }
                o.push('}');
            }
        }
    }
}

fn write_str(o: &mut String, s: &str) {
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(o, "\\u{:04x}", c as u32);
            }
            c => o.push(c),
        }
    }
    o.push('"');
}

struct P<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> P<'a> {
    fn ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\n' | b'\r' | b'\t') {
            self.i += 1;
        }
    }
    fn err<T>(&self, m: &str) -> Result<T, String> {
        Err(format!("JSON: {m} at byte {}", self.i))
    }
    fn lit(&mut self, s: &str, v: Json) -> Result<Json, String> {
        if self.b[self.i..].starts_with(s.as_bytes()) {
            self.i += s.len();
            Ok(v)
        } else {
            self.err("invalid literal")
        }
    }
    fn value(&mut self) -> Result<Json, String> {
        if self.i >= self.b.len() {
            return self.err("unexpected end");
        }
        match self.b[self.i] {
            b'{' => {
                self.i += 1;
                let mut m = Vec::new();
                self.ws();
                if self.b.get(self.i) == Some(&b'}') {
                    self.i += 1;
                    return Ok(Json::Obj(m));
                }
                loop {
                    self.ws();
                    let k = match self.value()? {
                        Json::Str(s) => s,
                        _ => return self.err("object key must be a string"),
                    };
                    self.ws();
                    if self.b.get(self.i) != Some(&b':') {
                        return self.err("expected ':'");
                    }
                    self.i += 1;
                    self.ws();
                    let v = self.value()?;
                    m.push((k, v));
                    self.ws();
                    match self.b.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b'}') => {
                            self.i += 1;
                            return Ok(Json::Obj(m));
                        }
                        _ => return self.err("expected ',' or '}'"),
                    }
                }
            }
            b'[' => {
                self.i += 1;
                let mut a = Vec::new();
                self.ws();
                if self.b.get(self.i) == Some(&b']') {
                    self.i += 1;
                    return Ok(Json::Arr(a));
                }
                loop {
                    self.ws();
                    a.push(self.value()?);
                    self.ws();
                    match self.b.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b']') => {
                            self.i += 1;
                            return Ok(Json::Arr(a));
                        }
                        _ => return self.err("expected ',' or ']'"),
                    }
                }
            }
            b'"' => {
                self.i += 1;
                let mut s = String::new();
                loop {
                    let Some(&c) = self.b.get(self.i) else {
                        return self.err("unterminated string");
                    };
                    self.i += 1;
                    match c {
                        b'"' => return Ok(Json::Str(s)),
                        b'\\' => {
                            let Some(&e) = self.b.get(self.i) else {
                                return self.err("bad escape");
                            };
                            self.i += 1;
                            match e {
                                b'"' => s.push('"'),
                                b'\\' => s.push('\\'),
                                b'/' => s.push('/'),
                                b'b' => s.push('\u{8}'),
                                b'f' => s.push('\u{c}'),
                                b'n' => s.push('\n'),
                                b'r' => s.push('\r'),
                                b't' => s.push('\t'),
                                b'u' => {
                                    let mut cp = self.hex4()?;
                                    if (0xD800..0xDC00).contains(&cp)
                                        && self.b[self.i..].starts_with(b"\\u")
                                    {
                                        self.i += 2;
                                        let lo = self.hex4()?;
                                        cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                                    }
                                    s.push(char::from_u32(cp).unwrap_or('\u{FFFD}'));
                                }
                                _ => return self.err("bad escape"),
                            }
                        }
                        _ => {
                            // copy a UTF-8 run
                            let start = self.i - 1;
                            let mut end = self.i;
                            while end < self.b.len() && self.b[end] != b'"' && self.b[end] != b'\\'
                            {
                                end += 1;
                            }
                            s.push_str(
                                std::str::from_utf8(&self.b[start..end])
                                    .map_err(|_| "JSON: invalid UTF-8".to_string())?,
                            );
                            self.i = end;
                        }
                    }
                }
            }
            b't' => self.lit("true", Json::Bool(true)),
            b'f' => self.lit("false", Json::Bool(false)),
            b'n' => self.lit("null", Json::Null),
            _ => {
                let st = self.i;
                while self.i < self.b.len()
                    && matches!(
                        self.b[self.i],
                        b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9'
                    )
                {
                    self.i += 1;
                }
                std::str::from_utf8(&self.b[st..self.i])
                    .ok()
                    .and_then(|t| t.parse::<f64>().ok())
                    .map(Json::Num)
                    .ok_or_else(|| format!("JSON: invalid number at byte {st}"))
            }
        }
    }
    fn hex4(&mut self) -> Result<u32, String> {
        let h = self
            .b
            .get(self.i..self.i + 4)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u32::from_str_radix(h, 16).ok());
        self.i += 4;
        h.ok_or_else(|| "JSON: bad \\u escape".to_string())
    }
}

/// Convenience constructors for building JSON output.
pub fn obj(v: Vec<(&str, Json)>) -> Json {
    Json::Obj(v.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}
pub fn s(v: &str) -> Json {
    Json::Str(v.to_string())
}
pub fn n(v: f64) -> Json {
    Json::Num(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip() {
        let src = r#"{"a": [1, 2.5, -3e2, true, null], "b": {"c": "é\n\"x\" é 😀"}, "d": []}"#;
        let j = Json::parse(src).unwrap();
        assert_eq!(j.get("a").unwrap().arr().unwrap()[2].num(), Some(-300.0));
        assert_eq!(j.get("b").unwrap().get_str("c"), Some("é\n\"x\" é 😀"));
        let again = Json::parse(&j.to_string_pretty()).unwrap();
        assert_eq!(j, again);
    }
}
