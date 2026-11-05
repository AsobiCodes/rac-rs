//! A small JSON reader for `mods/<id>/mod.json` (the engine has no JSON dependency): objects, arrays, strings with escapes,
//! numbers, booleans and null. Unknown keys are kept, so a newer mod.json with extra fields still parses.

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
	pub fn get(&self, key: &str) -> Option<&Json> {
		match self {
			Json::Obj(v) => v.iter().find(|(k, _)| k == key).map(|(_, v)| v),
			_ => None,
		}
	}

	pub fn as_str(&self) -> Option<&str> {
		match self {
			Json::Str(s) => Some(s),
			_ => None,
		}
	}

	pub fn as_f64(&self) -> Option<f64> {
		match self {
			Json::Num(n) => Some(*n),
			_ => None,
		}
	}

	pub fn as_array(&self) -> Option<&[Json]> {
		match self {
			Json::Arr(v) => Some(v),
			_ => None,
		}
	}
}

/// Parses one JSON document (a UTF-8 BOM is skipped); Err names the byte offset of the problem.
pub fn parse(text: &str) -> Result<Json, String> {
	let mut p = P { s: text.trim_start_matches('\u{feff}').as_bytes(), i: 0, depth: 0 };
	let v = p.value()?;
	p.ws();
	if p.i != p.s.len() {
		return Err(format!("trailing data at byte {}", p.i));
	}
	Ok(v)
}

struct P<'a> {
	s: &'a [u8],
	i: usize,
	depth: u32,
}

impl P<'_> {
	fn ws(&mut self) {
		while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
			self.i += 1;
		}
	}

	fn err<T>(&self, what: &str) -> Result<T, String> {
		Err(format!("{what} at byte {}", self.i))
	}

	fn eat(&mut self, c: u8) -> Result<(), String> {
		self.ws();
		if self.s.get(self.i) == Some(&c) {
			self.i += 1;
			Ok(())
		} else {
			self.err(&format!("expected '{}'", c as char))
		}
	}

	fn word(&mut self, w: &str, v: Json) -> Result<Json, String> {
		if self.s[self.i..].starts_with(w.as_bytes()) {
			self.i += w.len();
			Ok(v)
		} else {
			self.err("bad value")
		}
	}

	fn value(&mut self) -> Result<Json, String> {
		self.ws();
		match self.s.get(self.i) {
			Some(b'{') => self.nested(|p| p.object()),
			Some(b'[') => self.nested(|p| p.array()),
			Some(b'"') => self.string().map(Json::Str),
			Some(b't') => self.word("true", Json::Bool(true)),
			Some(b'f') => self.word("false", Json::Bool(false)),
			Some(b'n') => self.word("null", Json::Null),
			Some(c) if c.is_ascii_digit() || *c == b'-' => self.number(),
			_ => self.err("unexpected character"),
		}
	}

	fn nested(&mut self, f: impl FnOnce(&mut Self) -> Result<Json, String>) -> Result<Json, String> {
		self.depth += 1;
		if self.depth > 32 {
			return self.err("nesting too deep");
		}
		let r = f(self);
		self.depth -= 1;
		r
	}

	fn object(&mut self) -> Result<Json, String> {
		self.eat(b'{')?;
		let mut out = Vec::new();
		self.ws();
		if self.s.get(self.i) == Some(&b'}') {
			self.i += 1;
			return Ok(Json::Obj(out));
		}
		loop {
			self.ws();
			let k = self.string()?;
			self.eat(b':')?;
			out.push((k, self.value()?));
			self.ws();
			match self.s.get(self.i) {
				Some(b',') => self.i += 1,
				Some(b'}') => {
					self.i += 1;
					return Ok(Json::Obj(out));
				}
				_ => return self.err("expected ',' or '}'"),
			}
		}
	}

	fn array(&mut self) -> Result<Json, String> {
		self.eat(b'[')?;
		let mut out = Vec::new();
		self.ws();
		if self.s.get(self.i) == Some(&b']') {
			self.i += 1;
			return Ok(Json::Arr(out));
		}
		loop {
			out.push(self.value()?);
			self.ws();
			match self.s.get(self.i) {
				Some(b',') => self.i += 1,
				Some(b']') => {
					self.i += 1;
					return Ok(Json::Arr(out));
				}
				_ => return self.err("expected ',' or ']'"),
			}
		}
	}

	fn number(&mut self) -> Result<Json, String> {
		let start = self.i;
		while self.i < self.s.len() && matches!(self.s[self.i], b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E') {
			self.i += 1;
		}
		std::str::from_utf8(&self.s[start..self.i]).ok().and_then(|t| t.parse().ok()).map(Json::Num).ok_or_else(|| format!("bad number at byte {start}"))
	}

	fn hex4(&mut self) -> Result<u32, String> {
		let h = self.s.get(self.i..self.i + 4).and_then(|b| std::str::from_utf8(b).ok()).and_then(|t| u32::from_str_radix(t, 16).ok());
		self.i += 4;
		h.ok_or_else(|| format!("bad \\u escape at byte {}", self.i - 4))
	}

	fn string(&mut self) -> Result<String, String> {
		if self.s.get(self.i) != Some(&b'"') {
			return self.err("expected a string");
		}
		self.i += 1;
		let mut out = Vec::new();
		loop {
			let Some(&c) = self.s.get(self.i) else { return self.err("unterminated string") };
			self.i += 1;
			match c {
				b'"' => return String::from_utf8(out).map_err(|_| "string is not UTF-8".to_string()),
				b'\\' => {
					let Some(&e) = self.s.get(self.i) else { return self.err("unterminated escape") };
					self.i += 1;
					let ch = match e {
						b'"' | b'\\' | b'/' => e as char,
						b'n' => '\n',
						b't' => '\t',
						b'r' => '\r',
						b'b' => '\u{8}',
						b'f' => '\u{c}',
						b'u' => {
							let mut u = self.hex4()?;
							if (0xd800..0xdc00).contains(&u) && self.s[self.i..].starts_with(b"\\u") {
								self.i += 2;
								let lo = self.hex4()?;
								// A high surrogate needs a low one after it; anything else is one replacement character.
								u = if (0xdc00..0xe000).contains(&lo) { 0x10000 + ((u - 0xd800) << 10) + (lo - 0xdc00) } else { 0xfffd };
							}
							char::from_u32(u).unwrap_or('\u{fffd}')
						}
						_ => return self.err("bad escape"),
					};
					let mut buf = [0u8; 4];
					out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
				}
				_ => out.push(c),
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn parses_a_mod_json() {
		let j = parse("\u{feff}{\"format\": 1, \"id\": \"hd-pack\", \"serials\": [\"SCUS-97199\", \"SCES-50916\"], \"extra\": {\"a\": [null, true, -1.5e2]}, \"name\": \"Caf\\u00e9 \\\"HD\\\"\"}").unwrap();
		assert_eq!(j.get("format").and_then(Json::as_f64), Some(1.0));
		assert_eq!(j.get("id").and_then(Json::as_str), Some("hd-pack"));
		assert_eq!(j.get("serials").and_then(Json::as_array).map(|a| a.len()), Some(2));
		assert_eq!(j.get("name").and_then(Json::as_str), Some("Caf\u{e9} \"HD\""));
		assert_eq!(j.get("extra").and_then(|e| e.get("a")).and_then(Json::as_array).map(|a| a.len()), Some(3));
		assert_eq!(j.get("missing"), None);
	}

	#[test]
	fn surrogate_pairs_combine_and_a_bad_pair_is_one_replacement_character() {
		// `~` stands for a backslash in the JSON text.
		let (q, bs) = ('"', char::from(0x5c_u8).to_string());
		let text = |s: &str| parse(&format!("{q}{}{q}", s.replace('~', &bs))).unwrap().as_str().map(str::to_owned);
		assert_eq!(text("~ud83d~ude00"), char::from_u32(0x1f600).map(String::from));
		// A high surrogate followed by an escape that is no low surrogate: this subtracted below zero (a panic with overflow checks).
		assert_eq!(text("~ud800~u0041"), Some(char::REPLACEMENT_CHARACTER.to_string()));
		assert_eq!(text("~ud800"), Some(char::REPLACEMENT_CHARACTER.to_string()));
	}

	#[test]
	fn refuses_bad_json() {
		for bad in ["", "{", "{\"a\": }", "[1, 2", "{\"a\": 1} x", "{\"a\" 1}", "\"abc"] {
			assert!(parse(bad).is_err(), "{bad:?}");
		}
		assert!(parse(&"[".repeat(100)).is_err(), "depth limit");
	}
}
