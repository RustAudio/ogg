// Ogg decoder and encoder written in Rust
//
// Copyright (c) 2018 est31 <MTest31@outlook.com>
// and contributors. All rights reserved.
// Licensed under MIT license, or Apache 2 license,
// at your option. Please see the LICENSE file
// attached to this source distribution for details.

extern crate ogg;

use std::env;
use ogg::{PacketReader, PacketWriter};
use ogg::writing::PacketWriteEndInfo;
use std::fs::File;

fn main() {
	match run() {
		Ok(_) =>(),
		Err(err) => println!("Error: {}", err),
	}
}

macro_rules! btry {
	($e:expr) => {
		match $e {
			Ok(v) => v,
			Err(e) => {
				println!("Encountered Error: {:?}", e);
				break;
			},
		}
	};
}

fn run() -> Result<(), std::io::Error> {
	let input_path = env::args().nth(1).expect("No arg for input path found. Please specify a file to open.");
	let output_path = env::args().nth(2).expect("No arg for output path found. Please specify a file to save to.");
	println!("Opening file: {}", input_path);
	println!("Writing to: {}", output_path);
	let mut f_i = try!(File::open(input_path));
	let mut f_o = try!(File::create(output_path));
	let mut pck_rdr = PacketReader::new(&mut f_i);

	// This call doesn't discard anything as nothing has
	// been stored yet, but it does set bits that
	// make reading logic a bit more tolerant towards
	// errors.
	pck_rdr.delete_unread_packets();

	let mut pck_wtr = PacketWriter::new(&mut f_o);

	loop {
		let r = btry!(pck_rdr.read_packet());
		match r {
			Some(pck) => {
				let inf = if pck.last_in_stream() {
					PacketWriteEndInfo::EndStream
				} else if pck.last_in_page() {
					PacketWriteEndInfo::EndPage
				} else {
					PacketWriteEndInfo::NormalPacket
				};
				let stream_serial = pck.stream_serial();
				let absgp_page = pck.absgp_page();
				btry!(pck_wtr.write_packet(pck.data.into_boxed_slice(),
					stream_serial,
					inf,
					absgp_page));
			},
			// End of stream
			None => break,
		}
	}
	Ok(())
}
