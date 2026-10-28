//! A small JSON reader for the OmniLoader binding file (`omni/rac1.json`): objects, arrays, strings, numbers, true/false/null, and a writer
//! (`to_text`) for the one place that edits the binding (layout.rs). No streaming; the binding is a few KB. (rac-rs has no serde_json,
//! and one file this small does not need it.)

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
	pub fn str(&self) -> Option<&str> {
		match self {
			Json::Str(s) => Some(s),
			_ => None,
		}
	}
	pub fn arr(&self) -> &[Json] {
		match self {
			Json::Arr(v) => v,
			_ => &[],
		}
	}
	pub fn entries(&self) -> &[(String, Json)] {
		match self {
			Json::Obj(v) => v,
			_ => &[],
		}
	}
	pub fn bool(&self) -> bool {
		matches!(self, Json::Bool(true))
	}
	pub fn as_num(&self) -> Option<f64> {
		match self {
			Json::Num(n) => Some(*n),
			_ => None,
		}
	}

	/// Compact JSON text of this value (`parse` reads it back to an equal value).
	pub fn to_text(&self) -> String {
		let mut out = String::new();
		self.write(&mut out);
		out
	}

	fn write(&self, out: &mut String) {
		match self {
			Json::Null => out.push_str("null"),
			Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
			Json::Num(n) => out.push_str(&n.to_string()),
			Json::Str(s) => write_str(out, s),
			Json::Arr(v) => {
				out.push('[');
				for (i, x) in v.iter().enumerate() {
					if i > 0 {
						out.push(',');
					}
					x.write(out);
				}
				out.push(']');
			}
			Json::Obj(v) => {
				out.push('{');
				for (i, (k, x)) in v.iter().enumerate() {
					if i > 0 {
						out.push(',');
					}
					write_str(out, k);
					out.push(':');
					x.write(out);
				}
				out.push('}');
			}
		}
	}
}

fn write_str(out: &mut String, s: &str) {
	out.push('"');
	for c in s.chars() {
		match c {
			'"' => out.push_str("\\\""),
			'\\' => out.push_str("\\\\"),
			'\n' => out.push_str("\\n"),
			'\r' => out.push_str("\\r"),
			'\t' => out.push_str("\\t"),
			c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
			c => out.push(c),
		}
	}
	out.push('"');
}

pub fn parse(text: &str) -> Result<Json, String> {
	let mut p = P { b: text.trim_start_matches('\u{feff}').as_bytes(), i: 0, depth: 0 };
	let v = p.value()?;
	p.ws();
	if p.i != p.b.len() {
		return Err(format!("json: extra text at byte {}", p.i));
	}
	Ok(v)
}

/// How deep arrays and objects may nest: a file with thousands of `[` must be refused, not overflow the stack (the mods' manifests are read
/// at every start); real files nest a handful of levels.
const MAX_DEPTH: u32 = 128;

struct P<'a> {
	b: &'a [u8],
	i: usize,
	depth: u32,
}

impl P<'_> {
	fn ws(&mut self) {
		while self.i < self.b.len() && self.b[self.i].is_ascii_whitespace() {
			self.i += 1;
		}
	}
	fn eat(&mut self, c: u8) -> Result<(), String> {
		self.ws();
		if self.b.get(self.i) == Some(&c) {
			self.i += 1;
			Ok(())
		} else {
			Err(format!("json: expected '{}' at byte {}", c as char, self.i))
		}
	}
	fn lit(&mut self, word: &str, v: Json) -> Result<Json, String> {
		if self.b[self.i..].starts_with(word.as_bytes()) {
			self.i += word.len();
			Ok(v)
		} else {
			Err(format!("json: bad word at byte {}", self.i))
		}
	}
	fn value(&mut self) -> Result<Json, String> {
		if self.depth >= MAX_DEPTH {
			return Err(format!("json: nested deeper than {MAX_DEPTH} at byte {}", self.i));
		}
		self.depth += 1;
		let v = self.value_at();
		self.depth -= 1;
		v
	}
	fn value_at(&mut self) -> Result<Json, String> {
		self.ws();
		match self.b.get(self.i).copied() {
			None => Err("json: unexpected end".into()),
			Some(b'{') => {
				self.i += 1;
				let mut out = Vec::new();
				self.ws();
				if self.b.get(self.i) == Some(&b'}') {
					self.i += 1;
					return Ok(Json::Obj(out));
				}
				loop {
					self.ws();
					let k = self.string()?;
					self.eat(b':')?;
					out.push((k, self.value()?));
					self.ws();
					match self.b.get(self.i) {
						Some(b',') => self.i += 1,
						Some(b'}') => {
							self.i += 1;
							return Ok(Json::Obj(out));
						}
						_ => return Err(format!("json: expected ',' or '}}' at byte {}", self.i)),
					}
				}
			}
			Some(b'[') => {
				self.i += 1;
				let mut out = Vec::new();
				self.ws();
				if self.b.get(self.i) == Some(&b']') {
					self.i += 1;
					return Ok(Json::Arr(out));
				}
				loop {
					out.push(self.value()?);
					self.ws();
					match self.b.get(self.i) {
						Some(b',') => self.i += 1,
						Some(b']') => {
							self.i += 1;
							return Ok(Json::Arr(out));
						}
						_ => return Err(format!("json: expected ',' or ']' at byte {}", self.i)),
					}
				}
			}
			Some(b'"') => Ok(Json::Str(self.string()?)),
			Some(b't') => self.lit("true", Json::Bool(true)),
			Some(b'f') => self.lit("false", Json::Bool(false)),
			Some(b'n') => self.lit("null", Json::Null),
			Some(_) => {
				let s = self.i;
				while self.i < self.b.len() && matches!(self.b[self.i], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
					self.i += 1;
				}
				std::str::from_utf8(&self.b[s..self.i]).ok().and_then(|t| t.parse::<f64>().ok()).filter(|n| n.is_finite()).map(Json::Num).ok_or(format!("json: bad number at byte {s}"))
			}
		}
	}
	fn string(&mut self) -> Result<String, String> {
		if self.b.get(self.i) != Some(&b'"') {
			return Err(format!("json: expected a string at byte {}", self.i));
		}
		self.i += 1;
		let mut out: Vec<u8> = Vec::new();
		while let Some(&c) = self.b.get(self.i) {
			self.i += 1;
			match c {
				b'"' => return String::from_utf8(out).map_err(|_| "json: string is not UTF-8".to_string()),
				b'\\' => {
					let e = self.b.get(self.i).copied().ok_or("json: unfinished escape")?;
					self.i += 1;
					match e {
						b'n' => out.push(b'\n'),
						b't' => out.push(b'\t'),
						b'r' => out.push(b'\r'),
						b'b' => out.push(8),
						b'f' => out.push(12),
						b'u' => {
							let hex = self.b.get(self.i..self.i + 4).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u32::from_str_radix(h, 16).ok());
							self.i += 4;
							let ch = char::from_u32(hex.ok_or("json: bad \\u escape")?).unwrap_or('?');
							let mut buf = [0u8; 4];
							out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
						}
						other => out.push(other),
					}
				}
				_ => out.push(c),
			}
		}
		Err("json: unfinished string".into())
	}
}
