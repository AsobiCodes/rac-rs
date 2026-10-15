//! Bounds-checked little-endian reads. Every format reader goes through these so that bad data
//! becomes an error instead of a panic.

pub type R<T> = Result<T, String>;

pub fn span(data: &[u8], off: usize, size: usize) -> R<&[u8]> {
	off.checked_add(size)
		.filter(|&end| end <= data.len())
		.map(|end| &data[off..end])
		.ok_or_else(|| format!("range {off:#x}+{size:#x} exceeds {:#x} bytes", data.len()))
}

pub fn u16_at(d: &[u8], off: usize) -> R<u16> {
	let b = span(d, off, 2)?;
	Ok(u16::from_le_bytes([b[0], b[1]]))
}

pub fn i16_at(d: &[u8], off: usize) -> R<i16> {
	Ok(u16_at(d, off)? as i16)
}

pub fn u32_at(d: &[u8], off: usize) -> R<u32> {
	let b = span(d, off, 4)?;
	Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

pub fn i32_at(d: &[u8], off: usize) -> R<i32> {
	Ok(u32_at(d, off)? as i32)
}

pub fn f32_at(d: &[u8], off: usize) -> R<f32> {
	Ok(f32::from_bits(u32_at(d, off)?))
}
