// Ogg decoder and encoder written in Rust
//
// Copyright (c) 2016 est31 <MTest31@outlook.com>
// and contributors. All rights reserved.
// Licensed under MIT license, or Apache 2 license,
// at your option. Please see the LICENSE file
// attached to this source distribution for details.

fn main() {
	print_crc32_table();
}

fn get_tbl_elem(idx :u32) -> u32 {
	let mut r :u32 = idx << 24;
	for _ in 0..8 {
		r = (r << 1) ^ (-(((r >> 31) & 1) as i32) as u32 & 0x04c11db7);
	}
	return r;
}

fn print_crc32_table() {
	let mut lup_arr :[u32; 0x100] = [0; 0x100];
	for i in 0..0x100 {
		lup_arr[i] = get_tbl_elem(i as u32);
	}
	print_slice("CRC_LOOKUP_ARRAY", &lup_arr);
}

fn print_slice(name :&str, arr :&[u32]) {
	assert!(arr.len() > 4);
	println!("static {} : &[u32] = &[", name);
	let mut i :usize = 0;
	while i * 4 < arr.len() - 4 {
		println!("\t0x{:08x}, 0x{:08x}, 0x{:08x}, 0x{:08x},",
				arr[i * 4], arr[i * 4 + 1], arr[i * 4 + 2], arr[i * 4 + 3]);
		i += 1;
	}
	match arr.len() as i64 - i as i64 * 4 {
		1 => println!("\t0x{:08x}];", arr[i * 4]),
		2 => println!("\t0x{:08x}, 0x{:08x}];", arr[i * 4], arr[i * 4 + 1]),
		3 => println!("\t0x{:08x}, 0x{:08x}, 0x{:08x}];",
				arr[i * 4], arr[i * 4 + 1], arr[i * 4 + 2]),
		4 => println!("\t0x{:08x}, 0x{:08x}, 0x{:08x}, 0x{:08x}];",
				arr[i * 4], arr[i * 4 + 1], arr[i * 4 + 2], arr[i * 4 + 3]),
		de => panic!("impossible value {}", de),
	}
}
