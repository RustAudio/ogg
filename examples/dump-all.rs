// Ogg decoder and encoder written in Rust
//
// Copyright (c) 2018 est31 <MTest31@outlook.com>
// and contributors. All rights reserved.
// Licensed under MIT license, or Apache 2 license,
// at your option. Please see the LICENSE file
// attached to this source distribution for details.

extern crate ogg;

use std::env;
use ogg::{PacketReader, Packet};
use std::fs::File;

fn main() {
	match run() {
		Ok(_) =>(),
		Err(err) => println!("Error: {}", err),
	}
}

#[allow(dead_code)]
fn print_u8_slice(arr :&[u8]) {
	if arr.len() <= 4 {
		for a in arr {
			print!("0x{:02x} ", a);
		}
		println!("");
		return;
	}
	println!("[");
	let mut i :usize = 0;
	while i * 4 < arr.len() - 4 {
		println!("\t0x{:02x}, 0x{:02x}, 0x{:02x}, 0x{:02x},",
				arr[i * 4], arr[i * 4 + 1], arr[i * 4 + 2], arr[i * 4 + 3]);
		i += 1;
	}
	match arr.len() as i64 - i as i64 * 4 {
		1 => println!("\t0x{:02x}];", arr[i * 4]),
		2 => println!("\t0x{:02x}, 0x{:02x}];", arr[i * 4], arr[i * 4 + 1]),
		3 => println!("\t0x{:02x}, 0x{:02x}, 0x{:02x}];",
				arr[i * 4], arr[i * 4 + 1], arr[i * 4 + 2]),
		4 => println!("\t0x{:02x}, 0x{:02x}, 0x{:02x}, 0x{:02x}];",
				arr[i * 4], arr[i * 4 + 1], arr[i * 4 + 2], arr[i * 4 + 3]),
		de => panic!("impossible value {}", de),
	}
}

fn dump_pck_info(p :&Packet, ctr :usize) {
	println!("Packet: serial 0x{:08x}, data {:08} large, first {: >5}, last {: >5}, absgp 0x{:016x} nth {}",
		p.stream_serial(), p.data.len(), p.first_in_page(), p.last_in_page(),
		p.absgp_page(), ctr);
	print_u8_slice(&p.data);
}

fn run() -> Result<(), std::io::Error> {
	let file_path = env::args().nth(1).expect("No arg found. Please specify a file to open.");
	println!("Opening file: {}", file_path);
	let mut f = try!(File::open(file_path));
	let mut pck_rdr = PacketReader::new(&mut f);

	let mut ctr = 0;
	loop {
		let r = pck_rdr.read_packet();
		match r {
			Ok(Some(p)) => {
				dump_pck_info(&p, ctr);
				// Why do we not check p.last_packet here, and break the loop if false?
				// Well, first, this is only an example.
				// Second, the codecs may end streams in the middle of the file,
				// while still continuing other streams.
				// Therefore, don't do a probably too-early break.
				// Applications which know the codec may know after which
				// ended stream to stop decoding the file and thus not
				// encounter an error.
			},
			// End of stream
			Ok(None) => break,
			Err(e) => {
				println!("Encountered Error: {:?}", e);
				break;
			}
		}
		ctr+=1;
	}
	Ok(())
}
