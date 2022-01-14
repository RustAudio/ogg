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

// Lookup table to enable bytewise CRC32 calculation
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

#[cfg(test)]
pub fn vorbis_crc32(array :&[u8]) -> u32 {
	return vorbis_crc32_update(0, array);
}

pub fn vorbis_crc32_update(cur :u32, array :&[u8]) -> u32 {
	let mut ret :u32 = cur;
	for av in array {
		ret = (ret << 8) ^ CRC_LOOKUP_ARRAY[(*av as u32 ^ (ret >> 24)) as usize];
	}
	return ret;
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
