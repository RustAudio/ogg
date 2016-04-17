// Ogg decoder and encoder written in Rust
//
// Copyright (c) 2016 est31 <MTest31@outlook.com>
// and contributors. All rights reserved.
// Licensed under MIT license, or Apache 2 license,
// at your option. Please see the LICENSE file
// attached to this source distribution for details.

extern crate ogg;
extern crate byteorder;
extern crate time;

use std::env;
use ogg::{PacketReader, Packet};
use std::fs::File;
use time::precise_time_ns;

fn main() {
	match run() {
		Ok(_) =>(),
		Err(err) => println!("Error: {}", err),
	}
}

fn dump_pck_info(p :&Packet) {
	println!("Packet: serial 0x{:08x}, data {:08} large, last packet {: >5}, absgp 0x{:016x}",
		p.stream_serial, p.data.len(), p.last_packet, p.absgp_page);
}

fn run() -> Result<(), std::io::Error> {
	let file_path = env::args().nth(1).expect("No arg found. Please specify a file to open.");
	println!("Opening file: {}", file_path);
	let mut f = try!(File::open(file_path));
	let mut pck_rdr = PacketReader::new(&mut f);

	let mut byte_ctr :u64 = 0;
	let begin :u64 = precise_time_ns();

	loop {
		let r = pck_rdr.read_packet();
		match r {
			Ok(p) => {
				byte_ctr += p.data.len() as u64;
				dump_pck_info(&p);
				println!("speed: {} kb per ms ({} read)",
					1000. * (byte_ctr) as f32 / ((precise_time_ns() - begin) as f32),
					byte_ctr);
				// Why do we not check p.last_packet here, and break the loop if false?
				// Well, first, this is only an example.
				// Second, the codecs may end streams in the middle of the file,
				// while still continuing other streams.
				// Therefore, don't do a probably too-early break.
				// Applications which know the codec may know after which
				// ended stream to stop decoding the file and thus not
				// encounter an error.
			},
			Err(e) => {
				println!("Encountered Error: {:?}", e);
				break;
			}
		}
	}
	Ok(())
}
