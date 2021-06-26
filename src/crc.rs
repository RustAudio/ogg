// Ogg decoder and encoder written in Rust
//
// Copyright (c) 2016-2017 est31 <MTest31@outlook.com>
// and contributors. All rights reserved.
// Redistribution or use only under the terms
// specified in the LICENSE file attached to this
// source distribution.

/*!
Implementation of the CRC algorithm with the
vorbis specific parameters and setup
*/

use std::ops::{Add, Mul};

// Lookup table to enable bytewise CRC32 calculation
// Created using the crc32-table-generate example.
//
static CRC_LOOKUP_ARRAY : &[u32] = &[
	0x00000000, 0x04c11db7, 0x09823b6e, 0x0d4326d9,
	0x130476dc, 0x17c56b6b, 0x1a864db2, 0x1e475005,
	0x2608edb8, 0x22c9f00f, 0x2f8ad6d6, 0x2b4bcb61,
	0x350c9b64, 0x31cd86d3, 0x3c8ea00a, 0x384fbdbd,
	0x4c11db70, 0x48d0c6c7, 0x4593e01e, 0x4152fda9,
	0x5f15adac, 0x5bd4b01b, 0x569796c2, 0x52568b75,
	0x6a1936c8, 0x6ed82b7f, 0x639b0da6, 0x675a1011,
	0x791d4014, 0x7ddc5da3, 0x709f7b7a, 0x745e66cd,
	0x9823b6e0, 0x9ce2ab57, 0x91a18d8e, 0x95609039,
	0x8b27c03c, 0x8fe6dd8b, 0x82a5fb52, 0x8664e6e5,
	0xbe2b5b58, 0xbaea46ef, 0xb7a96036, 0xb3687d81,
	0xad2f2d84, 0xa9ee3033, 0xa4ad16ea, 0xa06c0b5d,
	0xd4326d90, 0xd0f37027, 0xddb056fe, 0xd9714b49,
	0xc7361b4c, 0xc3f706fb, 0xceb42022, 0xca753d95,
	0xf23a8028, 0xf6fb9d9f, 0xfbb8bb46, 0xff79a6f1,
	0xe13ef6f4, 0xe5ffeb43, 0xe8bccd9a, 0xec7dd02d,
	0x34867077, 0x30476dc0, 0x3d044b19, 0x39c556ae,
	0x278206ab, 0x23431b1c, 0x2e003dc5, 0x2ac12072,
	0x128e9dcf, 0x164f8078, 0x1b0ca6a1, 0x1fcdbb16,
	0x018aeb13, 0x054bf6a4, 0x0808d07d, 0x0cc9cdca,
	0x7897ab07, 0x7c56b6b0, 0x71159069, 0x75d48dde,
	0x6b93dddb, 0x6f52c06c, 0x6211e6b5, 0x66d0fb02,
	0x5e9f46bf, 0x5a5e5b08, 0x571d7dd1, 0x53dc6066,
	0x4d9b3063, 0x495a2dd4, 0x44190b0d, 0x40d816ba,
	0xaca5c697, 0xa864db20, 0xa527fdf9, 0xa1e6e04e,
	0xbfa1b04b, 0xbb60adfc, 0xb6238b25, 0xb2e29692,
	0x8aad2b2f, 0x8e6c3698, 0x832f1041, 0x87ee0df6,
	0x99a95df3, 0x9d684044, 0x902b669d, 0x94ea7b2a,
	0xe0b41de7, 0xe4750050, 0xe9362689, 0xedf73b3e,
	0xf3b06b3b, 0xf771768c, 0xfa325055, 0xfef34de2,
	0xc6bcf05f, 0xc27dede8, 0xcf3ecb31, 0xcbffd686,
	0xd5b88683, 0xd1799b34, 0xdc3abded, 0xd8fba05a,
	0x690ce0ee, 0x6dcdfd59, 0x608edb80, 0x644fc637,
	0x7a089632, 0x7ec98b85, 0x738aad5c, 0x774bb0eb,
	0x4f040d56, 0x4bc510e1, 0x46863638, 0x42472b8f,
	0x5c007b8a, 0x58c1663d, 0x558240e4, 0x51435d53,
	0x251d3b9e, 0x21dc2629, 0x2c9f00f0, 0x285e1d47,
	0x36194d42, 0x32d850f5, 0x3f9b762c, 0x3b5a6b9b,
	0x0315d626, 0x07d4cb91, 0x0a97ed48, 0x0e56f0ff,
	0x1011a0fa, 0x14d0bd4d, 0x19939b94, 0x1d528623,
	0xf12f560e, 0xf5ee4bb9, 0xf8ad6d60, 0xfc6c70d7,
	0xe22b20d2, 0xe6ea3d65, 0xeba91bbc, 0xef68060b,
	0xd727bbb6, 0xd3e6a601, 0xdea580d8, 0xda649d6f,
	0xc423cd6a, 0xc0e2d0dd, 0xcda1f604, 0xc960ebb3,
	0xbd3e8d7e, 0xb9ff90c9, 0xb4bcb610, 0xb07daba7,
	0xae3afba2, 0xaafbe615, 0xa7b8c0cc, 0xa379dd7b,
	0x9b3660c6, 0x9ff77d71, 0x92b45ba8, 0x9675461f,
	0x8832161a, 0x8cf30bad, 0x81b02d74, 0x857130c3,
	0x5d8a9099, 0x594b8d2e, 0x5408abf7, 0x50c9b640,
	0x4e8ee645, 0x4a4ffbf2, 0x470cdd2b, 0x43cdc09c,
	0x7b827d21, 0x7f436096, 0x7200464f, 0x76c15bf8,
	0x68860bfd, 0x6c47164a, 0x61043093, 0x65c52d24,
	0x119b4be9, 0x155a565e, 0x18197087, 0x1cd86d30,
	0x029f3d35, 0x065e2082, 0x0b1d065b, 0x0fdc1bec,
	0x3793a651, 0x3352bbe6, 0x3e119d3f, 0x3ad08088,
	0x2497d08d, 0x2056cd3a, 0x2d15ebe3, 0x29d4f654,
	0xc5a92679, 0xc1683bce, 0xcc2b1d17, 0xc8ea00a0,
	0xd6ad50a5, 0xd26c4d12, 0xdf2f6bcb, 0xdbee767c,
	0xe3a1cbc1, 0xe760d676, 0xea23f0af, 0xeee2ed18,
	0xf0a5bd1d, 0xf464a0aa, 0xf9278673, 0xfde69bc4,
	0x89b8fd09, 0x8d79e0be, 0x803ac667, 0x84fbdbd0,
	0x9abc8bd5, 0x9e7d9662, 0x933eb0bb, 0x97ffad0c,
	0xafb010b1, 0xab710d06, 0xa6322bdf, 0xa2f33668,
	0xbcb4666d, 0xb8757bda, 0xb5365d03, 0xb1f740b4];

/*
// Const implementation: TODO adopt it once MSRV > 1.46
static CRC_LOOKUP_ARRAY :&[u32] = &lookup_array();

const fn get_tbl_elem(idx :u32) -> u32 {
	let mut r :u32 = idx << 24;
	let mut i = 0;
	while i < 8 {
		r = (r << 1) ^ (-(((r >> 31) & 1) as i32) as u32 & 0x04c11db7);
		i += 1;
	}
	return r;
}

const fn lookup_array() -> [u32; 0x100] {
	let mut lup_arr :[u32; 0x100] = [0; 0x100];
	let mut i = 0;
	while i < 0x100 {
		lup_arr[i] = get_tbl_elem(i as u32);
		i += 1;
	}
	lup_arr
}
*/

/// An instance of polynomial quotient ring,
/// F_2[x] / x^32 + x^26 + x^23 + x^22 + x^16 + x^12 + x^11 + x^10 + x^8 + x^7 + x^5 + x^4 + x^2 + x + 1
/// represented as a 32-bit unsigned integer.
/// The i-th least significant bit corresponds to the coefficient of x^i.
///
/// This struct is introduced for the sake of readability.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Crc32(pub u32);

impl From<u32> for Crc32 {
	fn from(a: u32) -> Self {
		Crc32(a)
	}
}
impl From<Crc32> for u32 {
	fn from(a: Crc32) -> Self {
		a.0
	}
}

impl<C> Add<C> for Crc32 where C: Into<u32> {
	type Output = Crc32;

	fn add(self, rhs: C) -> Self {
		Crc32(self.0 ^ rhs.into())
	}
}

/// An array such that X_N[n] = x^n on Crc32.
const X_N: &[u32] = &[
	0x00000001, 0x00000002, 0x00000004, 0x00000008,
	0x00000010, 0x00000020, 0x00000040, 0x00000080,
	0x00000100, 0x00000200, 0x00000400, 0x00000800,
	0x00001000, 0x00002000, 0x00004000, 0x00008000,
	0x00010000, 0x00020000, 0x00040000, 0x00080000,
	0x00100000, 0x00200000, 0x00400000, 0x00800000,
	0x01000000, 0x02000000, 0x04000000, 0x08000000,
	0x10000000, 0x20000000, 0x40000000, 0x80000000,
	0x04c11db7, 0x09823b6e, 0x130476dc, 0x2608edb8,
	0x4c11db70, 0x9823b6e0, 0x34867077, 0x690ce0ee,
	0xd219c1dc, 0xa0f29e0f, 0x452421a9, 0x8a484352,
	0x10519b13, 0x20a33626, 0x41466c4c, 0x828cd898,
	0x01d8ac87, 0x03b1590e, 0x0762b21c, 0x0ec56438,
	0x1d8ac870, 0x3b1590e0, 0x762b21c0, 0xec564380,
	0xdc6d9ab7, 0xbc1a28d9, 0x7cf54c05, 0xf9ea980a,
	0xf7142da3, 0xeae946f1, 0xd1139055, 0xa6e63d1d,
];

impl<C> Mul<C> for Crc32 where C: Into<u32> {
	type Output = Crc32;
	fn mul(self, rhs: C) -> Self {
		// Very slow algorithm, so-called "grade-school multiplication".
		// Will be refined later.
		let mut ret = 0;
		let mut i = 0;
		let rhs = rhs.into();
		while i < 32 {
			let mut j = 0;
			while j < 32 {
				if (self.0 & 1 << i) != 0 && (rhs & 1 << j) != 0 {
					ret ^= X_N[i + j];
				}
				j += 1;
			}
			i += 1;
		}
		ret.into()
	}
}

impl Crc32 {
	/// Given a polynomial of degree 7 rhs, calculates self * x^8 + rhs * x^32.
	pub fn push(&self, rhs: u8) -> Self {
		let ret = (self.0 << 8) ^ CRC_LOOKUP_ARRAY[rhs as usize ^ (self.0 >> 24) as usize];
		ret.into()
	}

	/// Calculates self * x.
	pub fn mul_x(&self) -> Self {
		let (b, c) = self.0.overflowing_mul(2);
		let ret = b ^ (0u32.wrapping_sub(c as u32) & 0x04c11db7);
		ret.into()
	}

	/// Calculates self * x^8.
	pub fn mul_x8(&self) -> Self {
		self.push(0)
	}

	/// Given an integer n, calculates self * x^(8n) in a naive way.
	/// The time complexity is O(n), and may be slow for large n.
	pub fn mul_x8n(&self, mut n: usize) -> Self {
		let mut ret = *self;
		while n > 0 {
			ret = ret.mul_x8();
			n -= 1;
		}
		ret
	}
}

/// An array such that X8_2_N[n] = (x^8)^(2^n) on Crc32.
const X8_2_N: &[u32] = &[
	0x00000100, 0x00010000, 0x04c11db7, 0x490d678d,
	0xe8a45605, 0x75be46b7, 0xe6228b11, 0x567fddeb,
	0x88fe2237, 0x0e857e71, 0x7001e426, 0x075de2b2,
	0xf12a7f90, 0xf0b4a1c1, 0x58f46c0c, 0xc3395ade,
	0x96837f8c, 0x544037f9, 0x23b7b136, 0xb2e16ba8,
];

impl Crc32 {
	/// Given a non-negative integer n, calculates x^(8n).
	/// It must be n < 2^20, othrwise it panics.
	pub fn x_8n(mut n: usize) -> Crc32 {
		assert!(n < 1<<20);
		let mut ret = Crc32(1);
		let mut i = 0;
		while n > 0 {
			if n & 1 > 0 {
				ret = ret * X8_2_N[i];
			}
			n /= 2;
			i += 1;
		}
		ret
	}
}

#[cfg(test)]
pub fn vorbis_crc32(array :&[u8]) -> u32 {
	return vorbis_crc32_update(0, array);
}

pub fn vorbis_crc32_update(cur :u32, array :&[u8]) -> u32 {
	array.iter().fold(Crc32(cur), |cur, &x| cur.push(x)).0
}

#[test]
fn test_crc32() {
	// Test page taken from real Ogg file
	let test_arr = &[
	0x4f, 0x67, 0x67, 0x53, 0x00, 0x02, 0x00, 0x00,
	0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x74, 0xa3,
	0x90, 0x5b, 0x00, 0x00, 0x00, 0x00,
	// The spec requires us to zero out the CRC field
	/*0x6d, 0x94, 0x4e, 0x3d,*/ 0x00, 0x00, 0x00, 0x00,
	0x01, 0x1e, 0x01, 0x76, 0x6f, 0x72,
	0x62, 0x69, 0x73, 0x00, 0x00, 0x00, 0x00, 0x02,
	0x44, 0xac, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
	0x80, 0xb5, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00,
	0xb8, 0x01];
	println!();
	println!("CRC of \"==!\" calculated as 0x{:08x} (expected 0x9f858776)", vorbis_crc32(&[61,61,33]));
	println!("Test page CRC calculated as 0x{:08x} (expected 0x3d4e946d)", vorbis_crc32(test_arr));
	assert_eq!(vorbis_crc32(&[61,61,33]), 0x9f858776);
	assert_eq!(vorbis_crc32(test_arr), 0x3d4e946d);
	assert_eq!(vorbis_crc32(&test_arr[0 .. 27]), 0x7b374db8);
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_mul_x() {
		for (i, &a) in CRC_LOOKUP_ARRAY.iter().enumerate() {
			let mut x = Crc32::from(i as u32);
			for _ in 0..32 {
				x = x.mul_x();
			}
			assert_eq!(a, x.0);
		}
	}

	fn x_8n_naive(n: usize) -> Crc32 {
		let mut ret = Crc32(1);
		for _ in 0..n {
			ret = ret.push(0);
		}
		ret
	 }

	#[test]
	fn test_x_8n() {
		for i in 0..100 {
			assert_eq!(x_8n_naive(i), Crc32::x_8n(i));
		}
		assert_eq!(x_8n_naive(12345), Crc32::x_8n(12345));
	}

	#[test]
	fn test_mul_x8n() {
		let a = Crc32(0xa1b2c3d4);
		for i in 0..100 {
			assert_eq!(a * Crc32::x_8n(i), a.mul_x8n(i));
		}
	}
}
