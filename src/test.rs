// Ogg decoder and encoder written in Rust
//
// Copyright (c) 2016-2017 est31 <MTest31@outlook.com>
// and contributors. All rights reserved.
// Redistribution or use only under the terms
// specified in the LICENSE file attached to this
// source distribution.

use super::*;

use std::rc::Rc;
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

#[test]
fn test_ogg_packet_write() {
	let mut c = Cursor::new(Vec::new());

	// Test page taken from real Ogg file
	let test_arr_out = [
	0x4f, 0x67, 0x67, 0x53, 0x00, 0x02, 0x00, 0x00,
	0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x74, 0xa3,
	0x90, 0x5b, 0x00, 0x00, 0x00, 0x00, 0x6d, 0x94,
	0x4e, 0x3d, 0x01, 0x1e, 0x01, 0x76, 0x6f, 0x72,
	0x62, 0x69, 0x73, 0x00, 0x00, 0x00, 0x00, 0x02,
	0x44, 0xac, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
	0x80, 0xb5, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00,
	0xb8, 0x01u8];
	let test_arr_in = [0x01, 0x76, 0x6f, 0x72,
	0x62, 0x69, 0x73, 0x00, 0x00, 0x00, 0x00, 0x02,
	0x44, 0xac, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
	0x80, 0xb5, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00,
	0xb8, 0x01u8];

	{
		let mut w = PacketWriter::new(&mut c);
		w.write_packet(Rc::new(test_arr_in), 0x5b90a374,
			PacketWriteEndInfo::EndPage, 0).unwrap();
	}
	//print_u8_slice(c.get_ref());
	assert_eq!(c.get_ref().len(), test_arr_out.len());

	let cr = c.get_ref();
	test_arr_eq!(cr, test_arr_out);
}
