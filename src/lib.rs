// Ogg decoder and encoder written in Rust
//
// Copyright (c) 2016 est31 <MTest31@outlook.com>
// and contributors. All rights reserved.
// Redistribution or use only under the terms
// specified in the LICENSE file attached to this
// source distribution.

#![deny(unsafe_code)]
#![cfg_attr(test, deny(warnings))]
#![allow(unused_parens)] // To support C-style if's

#![cfg_attr(feature = "async", feature(specialization))]

/*!
Ogg container decoder and encoder

The most interesting structures for in this
mod are `PacketReader` and `PacketWriter`.
*/

extern crate byteorder;

#[cfg(test)]
use std::rc::Rc;
#[cfg(test)]
use std::io::{Cursor, Seek, SeekFrom};
macro_rules! test_arr_eq {
	($a_arr:expr, $b_arr:expr) => {
		for i in 0 .. $b_arr.len() {
			if $a_arr[i] != $b_arr[i] {
				panic!("Mismatch of values at index {}: {} {}", i, $a_arr[i], $b_arr[i]);
			}
		}
	}
}

#[cfg(feature = "async")]
mod buf_reader;

mod crc;
mod reading;
mod writing;

pub use writing::{PacketWriter, PacketWriteEndInfo};
pub use reading::{PacketReader, OggReadError};

#[cfg(feature = "async")]
pub use buf_reader::BufReader as BufReader;
#[cfg(feature = "async")]
pub use buf_reader::AdvanceAndSeekBack as AdvanceAndSeekBack;

/**
Ogg packet representation.

For the Ogg format, packets are the logically smallest subdivision it handles.

Every packet belongs to a *logical* bitstream. The *logical* bitstreams then form a *physical* bitstream, with the data combined in multiple different ways.

Every logical bitstream is identified by the serial number its pages have stored. The Packet struct contains a field for that number as well, so that one can find out which logical bitstream the Packet belongs to.
*/
pub struct Packet {
	/// The data the `Packet` contains
	pub data :Vec<u8>,
	/// `true` iff this packet is the first one in the logical bitstream.
	pub first_packet :bool,
	/// `true` iff this packet is the last one in the logical bitstream
	pub last_packet :bool,
	/// Absolute granule position of the last page the packet was in.
	/// The meaning of the absolute granule position is defined by the codec.
	pub absgp_page :u64,
	/// Serial number. Uniquely identifying the logical bitstream.
	pub stream_serial :u32,
	/*/// Packet counter
	/// Why u64? There are MAX_U32 pages, and every page has up to 128 packets. u32 wouldn't be sufficient here...
	pub sequence_num :u64,*/ // TODO perhaps add this later on...
}

#[test]
fn test_ogg_packet_rw() {
	let mut c = Cursor::new(Vec::new());
	let test_arr = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
	let test_arr_2 = [2, 4, 8, 16, 32, 64, 128, 127, 126, 125, 124];
	let test_arr_3 = [3, 5, 9, 17, 33, 65, 129, 129, 127, 126, 125];
	{
		let mut w = PacketWriter::new(&mut c);
		let np = PacketWriteEndInfo::NormalPacket;
		w.write_packet(Rc::new(test_arr), 0xdeadb33f, np, 0).unwrap();
		w.write_packet(Rc::new(test_arr_2), 0xdeadb33f, np, 1).unwrap();
		w.write_packet(Rc::new(test_arr_3), 0xdeadb33f,
			PacketWriteEndInfo::EndPage, 2).unwrap();
	}
	//print_u8_slice(c.get_ref());
	assert_eq!(c.seek(SeekFrom::Start(0)).unwrap(), 0);
	{
		let mut r = PacketReader::new(c);
		let p1 = r.read_packet().unwrap();
		assert_eq!(test_arr, *p1.data);
		let p2 = r.read_packet().unwrap();
		assert_eq!(test_arr_2, *p2.data);
		let p3 = r.read_packet().unwrap();
		assert_eq!(test_arr_3, *p3.data);
	}

	// Now test packets spanning multiple segments
	let mut c = Cursor::new(Vec::new());
	let test_arr = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
	let mut test_arr_2 = [0; 700];
	let test_arr_3 = [3, 5, 9, 17, 33, 65, 129, 129, 127, 126, 125];
	for (idx, a) in test_arr_2.iter_mut().enumerate() {
		*a = (idx as u8) / 4;
	}
	{
		let mut w = PacketWriter::new(&mut c);
		let np = PacketWriteEndInfo::NormalPacket;
		w.write_packet(Rc::new(test_arr), 0xdeadb33f, np, 0).unwrap();
		w.write_packet(Rc::new(test_arr_2), 0xdeadb33f, np, 1).unwrap();
		w.write_packet(Rc::new(test_arr_3), 0xdeadb33f,
			PacketWriteEndInfo::EndPage, 2).unwrap();
	}
	//print_u8_slice(c.get_ref());
	assert_eq!(c.seek(SeekFrom::Start(0)).unwrap(), 0);
	{
		let mut r = PacketReader::new(&mut c);
		let p1 = r.read_packet().unwrap();
		assert_eq!(test_arr, *p1.data);
		let p2 = r.read_packet().unwrap();
		test_arr_eq!(test_arr_2, *p2.data);
		let p3 = r.read_packet().unwrap();
		assert_eq!(test_arr_3, *p3.data);
	}

	// Now test packets spanning multiple pages
	let mut c = Cursor::new(Vec::new());
	let mut test_arr_2 = [0; 14_000];
	let test_arr_3 = [3, 5, 9, 17, 33, 65, 129, 129, 127, 126, 125];
	for (idx, a) in test_arr_2.iter_mut().enumerate() {
		*a = (idx as u8) / 4;
	}
	{
		let mut w = PacketWriter::new(&mut c);
		let np = PacketWriteEndInfo::NormalPacket;
		w.write_packet(Rc::new(test_arr_2), 0xdeadb33f, np, 1).unwrap();
		w.write_packet(Rc::new(test_arr_3), 0xdeadb33f,
			PacketWriteEndInfo::EndPage, 2).unwrap();
	}
	//print_u8_slice(c.get_ref());
	assert_eq!(c.seek(SeekFrom::Start(0)).unwrap(), 0);
	{
		let mut r = PacketReader::new(c);
		let p2 = r.read_packet().unwrap();
		test_arr_eq!(test_arr_2, *p2.data);
		let p3 = r.read_packet().unwrap();
		assert_eq!(test_arr_3, *p3.data);
	}
}

#[test]
fn test_page_end_after_first_packet() {
	// Test that everything works well if we force a page end
	// after the first packet
	let mut c = Cursor::new(Vec::new());
	let test_arr = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
	let test_arr_2 = [2, 4, 8, 16, 32, 64, 128, 127, 126, 125, 124];
	let test_arr_3 = [3, 5, 9, 17, 33, 65, 129, 129, 127, 126, 125];
	{
		let mut w = PacketWriter::new(&mut c);
		let np = PacketWriteEndInfo::NormalPacket;
		w.write_packet(Rc::new(test_arr), 0xdeadb33f,
			PacketWriteEndInfo::EndPage, 0).unwrap();
		w.write_packet(Rc::new(test_arr_2), 0xdeadb33f, np, 1).unwrap();
		w.write_packet(Rc::new(test_arr_3), 0xdeadb33f,
			PacketWriteEndInfo::EndPage, 2).unwrap();
	}
	//print_u8_slice(c.get_ref());
	assert_eq!(c.seek(SeekFrom::Start(0)).unwrap(), 0);
	{
		let mut r = PacketReader::new(c);
		let p1 = r.read_packet().unwrap();
		assert_eq!(test_arr, *p1.data);
		let p2 = r.read_packet().unwrap();
		assert_eq!(test_arr_2, *p2.data);
		let p3 = r.read_packet().unwrap();
		assert_eq!(test_arr_3, *p3.data);
	}
}
