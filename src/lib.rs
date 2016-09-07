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

use std::result;
use std::io;
use std::rc::Rc;
use std::io::{Cursor, Write, Seek, SeekFrom, Error};
use byteorder::{WriteBytesExt, ReadBytesExt, LittleEndian};
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::mem::replace;

mod buf_reader;

pub use buf_reader::BufReader as BufReader;
pub use buf_reader::AdvanceAndSeekBack as AdvanceAndSeekBack;

// Lookup table to enable bytewise CRC32 calculation
// Created using the crc32-table-generate example.
//
static CRC_LOOKUP_ARRAY : &'static[u32] = &[
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

#[allow(dead_code)]
fn vorbis_crc32(array :&[u8]) -> u32 {
	return vorbis_crc32_update(0, array);
}

fn vorbis_crc32_update(cur :u32, array :&[u8]) -> u32 {
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
	println!("");
	println!("CRC of \"==!\" calculated as 0x{:08x} (expected 0x9f858776)", vorbis_crc32(&[61,61,33]));
	println!("Test page CRC calculated as 0x{:08x} (expected 0x3d4e946d)", vorbis_crc32(test_arr));
	assert!(vorbis_crc32(&[61,61,33]) == 0x9f858776);
	assert!(vorbis_crc32(test_arr) == 0x3d4e946d);
	assert!(vorbis_crc32(&test_arr[0 .. 27]) == 0x7b374db8);
}

/// Error that can be raised when decoding an Ogg transport.
#[derive(Debug)]
pub enum OggReadError {
	/// The capture pattern for a new page was not found
	/// where one was expected.
	NoCapturePatternFound,
	/// Invalid stream structure version, with the given one
	/// attached.
	InvalidStreamStructVer(u8),
	/// Mismatch of the hash value with (expected, calculated) value.
	HashMismatch(u32, u32),
	/// I/O error occured.
	ReadError(std::io::Error),
	/// Some constraint required by the spec was not met.
	InvalidData,
}

impl std::error::Error for OggReadError {
	fn description(&self) -> &str {
		match self {
			&OggReadError::NoCapturePatternFound => "No Ogg capture pattern found",
			&OggReadError::InvalidStreamStructVer(_) =>
				"A non zero stream structure version was passed",
			&OggReadError::HashMismatch(_, _) => "CRC32 hash mismatch",
			&OggReadError::ReadError(_) => "I/O error",
			&OggReadError::InvalidData => "Constraint violated",
		}
	}

	fn cause(&self) -> Option<&std::error::Error> {
		match self {
			&OggReadError::ReadError(ref err) => Some(err as &std::error::Error),
			_ => None
		}
	}
}

impl std::fmt::Display for OggReadError {
	fn fmt(&self, fmt :&mut std::fmt::Formatter) -> Result<(), std::fmt::Error> {
		write!(fmt, "{}", std::error::Error::description(self))
	}
}

impl From<std::io::Error> for OggReadError {
	fn from(err :std::io::Error) -> OggReadError {
		return OggReadError::ReadError(err);
	}
}

/// Ogg version of the `std::io::Result` type.
///
/// We need `std::result::Result` at other points
/// too, so we can't use `Result` as the name.
pub type IoResult<T> = result::Result<T, std::io::Error>;

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

/// Internal helper struct for PacketReader state
struct PageInfo {
	/// `true` is the first packet is continued from the page before and `false` if its a "fresh" one
	starts_with_continued :bool,
	/// `true` if this page is the first one in the logical bitstream
	first_page :bool,
	/// `true` if this page is the last one in the logical bitstream
	last_page :bool,
	/// Absolute granule position of last read page. The codec defines further meaning.
	last_absgp :u64,
	/// Page counter of last page read
	sequence_num :u32,

	/// Packet information:
	/// index is number of packet,
	/// tuple is (offset, length) of packet
	/// if ends_with_continued is true, the last element will contain information
	/// about the continued packet
	packet_positions :Vec<(u16,u16)>,
	/// `true` if the last packet is continued in subsequent page(s)
	/// `false` if the last packet has a segment of length < 255 inside this page
	ends_with_continued :bool,
	/// The index of the first "unread" packet
	packet_idx :u8,
	/// Contains the package data
	page_body :Vec<u8>,

	/// If there is a residue from previous pages in terms of a package spanning multiple
	/// pages, this field contains it. Having this Vec<Vec<u8>> and
	/// not Vec<u8> ensures to give us O(n) complexity, not O(n^2)
	/// for `n` as number of pages that the packet is contained in.
	last_overlap_pck :Vec<Vec<u8>>,
}

impl PageInfo {
	/// Returns `true` if the first "unread" packet is the first one
	/// in the page, `false` otherwise.
	fn is_first_pck_in_pg(self :&PageInfo) -> bool {
		return self.packet_idx == 0;
	}
	/// Returns `true` if the first "unread" packet is the last one
	/// in the page, `false` otherwise.
	/// If the first "unread" packet isn't completed in this page
	/// (spans page borders), this returns `false`.
	fn is_last_pck_in_pg(self :&PageInfo) -> bool {
		return ((self.packet_idx + 1 + (self.ends_with_continued as u8)) as usize
			== self.packet_positions.len());
	}
}

/**
Reader for packets from an Ogg stream.

This reads codec packets belonging to several different logical streams from one physical Ogg container stream.

If the `async` feature is activated, and you pass as internal reader a valid implementation of the
`AdvanceAndSeekBack` trait, like the `BufReader` wrapper, the PacketReader will support async operation,
meaning that its internal state doesn't get corrupted if from multiple consecutive reads which it performs,
some fail with e.g. the WouldBlock error kind.

Please note that if the `async` feature is not activated, and the BufReader (or any implementation of the trait)
is used, it will still appear to work, but it will have leaky behaviour, with performance degrading the more has
been read from the stream, and it may even panic in some edge cases. Therefore, you need to activate the async
feature.
*/
pub struct PacketReader<'a, T :io::Read + io::Seek + 'a> {
	rdr :&'a mut T,

	// TODO the hashmap plus the set is perhaps smart ass perfect design but could be made more performant I guess...
	// I mean: in > 99% of all cases we'll just have one or two streams.
	// AND: their setup changes only very rarely.

	/// Contains info about all logical streams that
	page_infos :HashMap<u32, PageInfo>,

	/// Contains the stream_serial of the stream that contains some unprocessed packet data.
	/// There is always <= 1, bc if there is one, no new pages will be read, so there is no chance for a second to be added
	/// None if there is no such stream and one has to read a new page.
	stream_with_stuff :Option<u32>,

	// Bool that is set to true when a seek of the stream has occured.
	// This helps validator code to decide whether to accept certain strange data.
	has_seeked :bool,
}

impl<'a, T :io::Read + io::Seek + 'a> PacketReader <'a, T> {
	/// Constructs a new `PacketReader` with a given `Read`.
	pub fn new(rdr :&mut T) -> PacketReader<T> {
		return PacketReader { rdr: rdr, page_infos: HashMap::new(),
			stream_with_stuff: None, has_seeked: false };
	}
	/// Reads a packet, and returns it on success.
	pub fn read_packet(self :&mut PacketReader <'a, T>) -> Result<Packet, OggReadError> {
		while self.stream_with_stuff == None {
			try!(self.read_ogg_page());
		}
		let str_serial :u32 = self.stream_with_stuff.unwrap();
		let mut pg_info = self.page_infos.get_mut(&str_serial).unwrap();
		let (offs, len) = pg_info.packet_positions[pg_info.packet_idx as usize];
		// If there is a continued packet, and we are at the start right now,
		// and we actually have its end in the current page, glue it together.
		let packet_content :Vec<u8> = if (
				pg_info.packet_idx == 0 && pg_info.starts_with_continued
				&& !(pg_info.ends_with_continued && pg_info.packet_positions.len() == 1)
		) {
			// First find out the size of our spanning packet
			let mut siz :usize = 0;
			for pck in pg_info.last_overlap_pck.iter() {
				siz += pck.len();
			}
			siz += len as usize;
			let mut cont :Vec<u8> = Vec::with_capacity(siz);

			// Then do the copying
			for pck in pg_info.last_overlap_pck.iter() {
				cont.write_all(pck).unwrap();
			}
			// Now reset the overlap container again
			pg_info.last_overlap_pck = Vec::new();
			cont.write_all(&pg_info.page_body[offs as usize .. (offs + len) as usize]).unwrap();

			cont
		} else {
			let mut cont :Vec<u8> = Vec::with_capacity(len as usize);
			// TODO The copy below is totally unneccessary. It is only needed so that we don't have to carry around the old Vec's.
			// TODO get something like the shared_slice crate for RefCells, so that we can also have mutable data, shared through
			// slices.
			let cont_slice :&[u8] = &pg_info.page_body[offs as usize .. (offs + len) as usize];
			cont.write_all(cont_slice).unwrap();
			cont
		};

		let first_pck_in_pg = pg_info.is_first_pck_in_pg();
		let first_pck_overall = pg_info.first_page && first_pck_in_pg;

		let last_pck_in_pg = pg_info.is_last_pck_in_pg();
		let last_pck_overall = pg_info.last_page && last_pck_in_pg;

		// Update the last read index.
		pg_info.packet_idx += 1;
		// Set stream_with_stuff to None so that future packet reads
		// yield a page read first
		if last_pck_in_pg {
			self.stream_with_stuff = None;
		}

		return Ok(Packet {
			data: packet_content,
			first_packet: first_pck_overall,
			last_packet: last_pck_overall,
			absgp_page: pg_info.last_absgp,
			stream_serial: str_serial,
		});
	}

	/// Reads until the new page header, and then returns the page header array.
	///
	/// If no new page header is immediately found, it performs a "recapture",
	/// meaning it searches for the capture pattern, and if it finds it, it
	/// reads the complete first 27 bytes of the header, and returns them.
	fn read_until_pg_header(self :&mut PacketReader <'a, T>) -> Result<[u8; 27], OggReadError> {
		let mut cpat_offs = 0;
		// Returns the Some(off), where off is the offset of the last byte
		// of the capture pattern if its found, None if the capture pattern
		// is not inside the passed slice.
		let mut check_arr = |arr :&[u8]| {
			for (i, ch) in arr.iter().enumerate() {
				match *ch {
					0x4f /*'O'*/ if cpat_offs == 0 => cpat_offs = 1,
					0x67 /*'g'*/ if cpat_offs == 1 || cpat_offs == 2 => cpat_offs += 1,
					0x53 /*'S'*/ if cpat_offs == 3 => return Some(i),
					_ => cpat_offs = 0,
				}
			}
			return None;
		};
		// First try the most normal case: The capture
		// pattern is at offset 0. No need for "recapture".
		let mut header_buf :[u8; 27] = [0; 27];
		try!(self.rdr.read_exact(&mut header_buf));
		match check_arr(&header_buf) {
			Some(idx) => if idx == 3 {
				// No need for recapture
				return Ok(header_buf);
			} else {
				// Capture pattern found, but offset by
				// a small amount.
				// Read remaining parts of the header,
				// then reassemble and return
				let mut ret_buf :[u8; 27] = [0; 27];
				let tocopy_len = header_buf.len() - idx;
				(&mut ret_buf[0..tocopy_len]).copy_from_slice(&header_buf[idx..]);
				try!(self.rdr.read_exact(&mut ret_buf[tocopy_len..]));
				return Ok(ret_buf);
			},
			None => (), // Not found, do nothing.
		}
		let mut read_amount = 27;
		// 150 kb gives us a bit of safety: we can survive
		// up to one page with a corrupted capture pattern
		// after having seeked right after a capture pattern
		// of an earlier page.
		let read_amount_max = 150 * 1024;
		let mut rdr_buf :[u8; 1024] = [0; 1024];
		loop {
			let rd_len = try!(self.rdr.read(&mut rdr_buf));
			read_amount += rd_len;
			if rd_len == 0 {
				// Reached EOF.
				try!(Err(OggReadError::NoCapturePatternFound));
			}
			match check_arr(&rdr_buf[0..rd_len]) {
				Some(off) => {
					let mut ret_buf :[u8; 27] = [0; 27];
					ret_buf[0] = 0x4f; // 'O'
					ret_buf[1] = 0x67; // 'g'
					ret_buf[2] = 0x67; // 'g'
					ret_buf[3] = 0x53; // 'S' (Not actually needed)
					let cnt_from_rdr_buf = rdr_buf.len() - off;
					use std::cmp::min;
					let copy_amount = min(24, cnt_from_rdr_buf);
					(&mut ret_buf[3..copy_amount + 3])
						.copy_from_slice(&rdr_buf[off..copy_amount + off]);
					if cnt_from_rdr_buf > 24 {
						// We have read too much content (exceeding the header).
						// Seek back so that we are at the position
						// right after the header.
						try!(self.seek_back(cnt_from_rdr_buf - 24));
					} else if cnt_from_rdr_buf == 24 {
						// All is fine. Nothing has to be done.
					} else {
						// We still have to read some content.
						try!(self.rdr.read_exact(
							&mut ret_buf[cnt_from_rdr_buf + 3..]));
					}
					return Ok(ret_buf);
				},
				None => (), // Nothing found.
			}
			if read_amount > read_amount_max {
				// Exhaustive searching for the capture pattern
				// has returned no ogg capture pattern.
				try!(Err(OggReadError::NoCapturePatternFound));
			}
		}
	}

	/// Reads a new Ogg page.
	///
	/// This method reads a new Ogg page.
	///
	/// To support seeking this does not assume that the capture pattern
	/// is at the current reader position.
	/// Instead it searches until it finds the capture pattern.
	fn read_ogg_page(self :&mut PacketReader <'a, T>) -> Result<(), OggReadError> {
		let mut header_buf :[u8; 27] = try!(self.read_until_pg_header());
		let mut header_rdr = Cursor::new(header_buf);
		header_rdr.set_position(4);
		let stream_structure_version = try!(header_rdr.read_u8());
		if stream_structure_version != 0 {
			try!(Err(OggReadError::InvalidStreamStructVer(stream_structure_version)));
		}
		let header_type_flag = try!(header_rdr.read_u8());
		let absgp = try!(header_rdr.read_u64::<LittleEndian>());
		let stream_serial = try!(header_rdr.read_u32::<LittleEndian>());
		let sequence_num = try!(header_rdr.read_u32::<LittleEndian>());
		let checksum = try!(header_rdr.read_u32::<LittleEndian>());
		let page_segments :usize = try!(header_rdr.read_u8()) as usize;

		let continued_packet :bool = header_type_flag & 0x01u8 != 0;
		let first_page :bool = header_type_flag & 0x02u8 != 0;
		let last_page :bool = header_type_flag & 0x04u8 != 0;

		let mut segments_buf = vec![0; page_segments]; // TODO fix this, we initialize memory for NOTHING!!! Out of some reason, this is seen as "unsafe" by rustc.
		try!(self.rdr.read_exact(&mut segments_buf));

		let mut page_siz :u16 = 0; // Size of the page's body
		let mut packet_count :u16 = 0; // Number of packet ending segments
		let mut ends_with_continued :bool = continued_packet;

		// First run: get the number of packets
		// whether the page ends with a continued packet
		// and the size of the page's body
		for val in &segments_buf {
			page_siz += *val as u16;
			// Increment by 1 if val < 255, otherwise by 0
			packet_count += (*val < 255) as u16;
			ends_with_continued = !(*val < 255);
		}

		let mut packets = Vec::with_capacity(packet_count as usize
			+ ends_with_continued as usize);
		let mut cur_packet_siz :u16 = 0;
		let mut cur_packet_offs :u16 = 0;

		// Second run: get the offsets of the packets
		// Not that we need it right now, but its much more fun this way, am I right
		for val in &segments_buf {
			cur_packet_siz += *val as u16;
			if *val < 255 {
				packets.push((cur_packet_offs, cur_packet_siz));
				cur_packet_offs += cur_packet_siz;
				cur_packet_siz = 0;
			}
		}
		if ends_with_continued {
			packets.push((cur_packet_offs, cur_packet_siz));
		}

		let mut page_data = vec![0; page_siz as usize];
		try!(self.rdr.read_exact(&mut page_data));

		self.maybe_advance();

		// Now to hash calculation.
		// 1. Clear the header buffer
		header_buf[22] = 0;
		header_buf[23] = 0;
		header_buf[24] = 0;
		header_buf[25] = 0;

		// 2. Calculate the hash
		let mut hash_calculated :u32;
		hash_calculated = vorbis_crc32_update(0, &header_buf);
		hash_calculated = vorbis_crc32_update(hash_calculated, &segments_buf);
		hash_calculated = vorbis_crc32_update(hash_calculated, &page_data);

		// 3. Compare to the extracted one
		if checksum != hash_calculated {
			try!(Err(OggReadError::HashMismatch(checksum, hash_calculated)));
		}

		match self.page_infos.entry(stream_serial) {
			Entry::Occupied(mut o) => {
				let inf = o.get_mut();
				if first_page {
					try!(Err(OggReadError::InvalidData));
				}
				if continued_packet != inf.ends_with_continued {
					if !self.has_seeked {
						try!(Err(OggReadError::InvalidData));
					} else {
						// If we have seeked, we are more tolerant here,
						// and just drop the continued packet's content.

						inf.last_overlap_pck.clear();
						if continued_packet {
							packets.remove(0);
							if packet_count != 0 {
								// Decrease packet count by one. Normal case.
								packet_count -= 1;
							} else {
								// If the packet count is 0, this means
								// that we start and end with the same continued packet.
								// So now as we ignore that packet, we must clear the
								// ends_with_continued state as well.
								ends_with_continued = false;
							}
						}
					}
				} else if continued_packet {
					// Remember the packet at the end so that it can be glued together once
					// we encounter the next segment with length < 255 (doesnt have to be in this page)
					let (offs, len) = inf.packet_positions[inf.packet_idx as usize];
					if len as usize != inf.page_body.len() {
						let mut tmp = Vec::with_capacity(len as usize);
						tmp.write_all(&inf.page_body[offs as usize .. (offs + len) as usize]).unwrap();
						inf.last_overlap_pck.push(tmp);
					} else {
						// Little optimisation: don't copy if not neccessary
						inf.last_overlap_pck.push(replace(&mut inf.page_body, vec![0;0]));
					}

				}
				inf.starts_with_continued = continued_packet;
				inf.first_page = first_page;
				inf.last_page = last_page;
				inf.last_absgp = absgp;
				inf.sequence_num = sequence_num;
				inf.packet_positions = packets;
				inf.ends_with_continued = ends_with_continued;
				inf.packet_idx = 0;
				inf.page_body = page_data;
			},
			Entry::Vacant(v) => {
				if !self.has_seeked {
					if (!first_page || continued_packet) {
						// If we haven't seeked, this is an error.
						try!(Err(OggReadError::InvalidData));
					}
				} else {
					if !first_page {
						// we can just ignore this.
					}
					if continued_packet {
						// Ignore the continued packet's content.
						// This is a normal occurence if we have just seeked.
						packets.remove(0);
						if packet_count != 0 {
							// Decrease packet count by one. Normal case.
							packet_count -= 1;
						} else {
							// If the packet count is 0, this means
							// that we start and end with the same continued packet.
							// So now as we ignore that packet, we must clear the
							// ends_with_continued state as well.
							ends_with_continued = false;
						}
					}
				}
				v.insert(PageInfo {
					starts_with_continued: continued_packet,
					first_page: first_page,
					last_page: last_page,
					last_absgp: absgp,
					sequence_num: sequence_num,
					packet_positions: packets,
					ends_with_continued: ends_with_continued,
					packet_idx: 0,
					page_body: page_data,
					last_overlap_pck: Vec::new(),
				});
			},
		}
		let pg_has_stuff :bool = packet_count > 0;

		if pg_has_stuff {
			self.stream_with_stuff = Some(stream_serial);
		} else {
			self.stream_with_stuff = None;
		}

		return Ok(());
	}

	#[cfg(feature = "async")]
	fn maybe_advance(&mut self) {
		trait MaybeAdvance {
			fn maybe_advance(&mut self);
		}
		impl<T> MaybeAdvance for T {
			default fn maybe_advance(&mut self) { }
		}
		impl<T :AdvanceAndSeekBack> MaybeAdvance for T {
			fn maybe_advance(&mut self) {
				self.advance();
			}
		}
		self.rdr.maybe_advance();
	}

	#[cfg(not(feature = "async"))]
	fn maybe_advance(&mut self) {
		// Do nothing ...
	}

	#[cfg(feature = "async")]
	fn seek_back(&mut self, len :usize) -> io::Result<()> {
		trait MaybeAdvance {
			fn seek_back(&mut self, len :usize) -> io::Result<()>;
		}
		impl<T :Seek> MaybeAdvance for T {
			default fn seek_back(&mut self, len :usize) -> io::Result<()> {
				return match self.seek(SeekFrom::Current(-(len as i64))) {
					Ok(_) => Ok(()),
					Err(e) => Err(e),
				};
			}
		}
		impl<T :Seek + AdvanceAndSeekBack> MaybeAdvance for T {
			fn seek_back(&mut self, len :usize) -> io::Result<()> {
				return self.seek_back(len);
			}
		}
		self.rdr.maybe_advance(len);
	}

	#[cfg(not(feature = "async"))]
	fn seek_back(&mut self, len :usize) -> io::Result<()> {
		return match self.rdr.seek(SeekFrom::Current(-(len as i64))) {
			Ok(_) => Ok(()),
			Err(e) => Err(e),
		};
	}

	/// Seeks the underlying reader
	///
	/// Seeks the reader that this PacketReader bases on by the specified
	/// number of bytes. All new pages will be read from the new position.
	///
	/// This also flushes all the unread packets in the queue.
	pub fn seek_bytes(&mut self, pos :SeekFrom) -> Result<u64, Error> {
		let r = try!(self.rdr.seek(pos));
		// Reset the internal state
		self.stream_with_stuff = None;
		self.page_infos = HashMap::new();
		self.has_seeked = true;
		return Ok(r);
	}
}

/**
Writer for packets into an Ogg stream.

Note that the functionality of this struct isn't as well tested as for
the `PacketReader` struct.
*/
pub struct PacketWriter<'a, T :io::Write + 'a> {
	wtr :&'a mut T,

	page_vals :HashMap<u32, CurrentPageValues>,
}

struct CurrentPageValues {
	/// `true` if this page is the first one in the logical bitstream
	first_page :bool,
	/// Page counter of the current page
	/// Increased for every page
	sequence_num :u32,

	/// Points to the first unwritten position in cur_pg_lacing.
	segment_cnt :u8,
	cur_pg_lacing :[u8; 255],
	cur_pg_data :Vec<Rc<[u8]>>,

	/// Some(offs), if the last packet
	/// couldn't make it fully into this page, and
	/// has to be continued in the next page.
	///
	/// `offs` should point to the first idx in
	/// cur_pg_data[last] that should NOT be written
	/// in this page anymore.
	///
	/// None if all packets can be written nicely.
	pck_this_overflow_idx :Option<usize>,

	/// Some(offs), if the first packet
	/// couldn't make it fully into the last page, and
	/// has to be continued in this page.
	///
	/// `offs` should point to the first idx in cur_pg_data[0]
	/// that hasn't been written.
	///
	/// None if all packets can be written nicely.
	pck_last_overflow_idx :Option<usize>,
}

/// Specifies whether to end something with the write of the packet.
///
/// If you want to end a stream you need to inform the Ogg `PacketWriter`
/// about this. This is the enum to do so.
///
/// Also, Codecs sometimes have special requirements to put
/// the first packet of the whole stream into its own page.
/// The `EndPage` variant can be used for this.
#[derive(PartialEq)]
#[derive(Clone, Copy)]
pub enum PacketWriteEndInfo {
	/// No ends here, just a normal packet
	NormalPacket,
	/// Force-end the current page
	EndPage,
	/// End the whole logical stream.
	EndStream,
}

impl <'a, T :io::Write + 'a> PacketWriter<'a, T> {
	pub fn new(wtr :&mut T) -> PacketWriter<T> {
		return PacketWriter {
			wtr : wtr,
			page_vals : HashMap::new(),
		};
	}
	/// Write a packet
	///
	///
	pub fn write_packet(&mut self, pck_cont :Rc<[u8]>, serial :u32,
			inf :PacketWriteEndInfo,
			/* TODO find a better way to design the API around
				passing the absgp to the underlying implementation.
				e.g. the caller passes a closure on init which gets
				called when we encounter a new page... with the param
				the index inside the current page, or something.
			*/
			absgp :u64) -> IoResult<()> {
		let is_end_stream :bool = inf == PacketWriteEndInfo::EndStream;
		let mut pg = self.page_vals.entry(serial).or_insert(
			CurrentPageValues {
				first_page : true,
				sequence_num : 0,
				segment_cnt : 0,
				cur_pg_lacing :[0; 255],
				cur_pg_data :Vec::with_capacity(255),
				pck_this_overflow_idx : None,
				pck_last_overflow_idx : None,
			}
		);

		pg.cur_pg_data.push(pck_cont.clone());

		let last_data_segment_size = (pck_cont.len() % 255) as u8;
		let needed_segments :usize = (pck_cont.len() / 255) + 1;
		let mut segment_in_page_i :u8 = pg.segment_cnt;
		let mut at_page_end :bool = false;
		for segment_i in 0 .. needed_segments {
			at_page_end = false;
			if segment_i + 1 < needed_segments {
				// For all segments containing 255 pieces of data
				pg.cur_pg_lacing[segment_in_page_i as usize] = 255;
			} else {
				// For the last segment, must contain < 255 pieces of data
				// (including 0)
				pg.cur_pg_lacing[segment_in_page_i as usize] = last_data_segment_size;
			}
			segment_in_page_i = segment_in_page_i.wrapping_add(1);
			if segment_in_page_i == 0 {
				if segment_i + 1 < needed_segments {
					// We have to flush a page, but we know there are more to come...
					pg.pck_this_overflow_idx = Some((segment_i + 1) * 255);
					try!(PacketWriter::write_page(self.wtr, serial, pg, false, absgp));
				} else {
					// We have to write a page end, and its the very last in the stream
					try!(PacketWriter::write_page(self.wtr,
						serial, pg, is_end_stream, absgp));
					// Not actually required either
					// (it is always None except if we set it to Some directly
					// before we call write_page)
					pg.pck_this_overflow_idx = None;
					// Required (it could have been Some(offs) before)
					pg.pck_last_overflow_idx = None;
				}
				at_page_end = true;
			}
			pg.segment_cnt = segment_in_page_i;
		}
		if (inf != PacketWriteEndInfo::NormalPacket) && !at_page_end {
			// Write a page end
			try!(PacketWriter::write_page(self.wtr, serial, pg, is_end_stream, absgp));

			pg.pck_last_overflow_idx = None;

			// TODO if inf was PacketWriteEndInfo::EndStream, we have to
			// somehow erase pg from the hashmap...
			// any ideas? perhaps needs external scope...
		}
		// All went fine.
		Ok(())
	}
	fn write_page(wtr :&'a mut T, serial :u32, pg :&mut CurrentPageValues,
			last_page :bool, absgp :u64)  -> IoResult<()> {
		{
			// The page header with everything but the lacing values:
			let mut hdr_cur = Cursor::new(Vec::with_capacity(27));
			try!(hdr_cur.write_all(&[0x4f, 0x67, 0x67, 0x53, 0x00]));
			let mut flags :u8 = 0;
			if pg.pck_last_overflow_idx.is_some() { flags |= 0x01; }
			if pg.first_page { flags |= 0x02; }
			if last_page { flags |= 0x04; }
			try!(hdr_cur.write_u8(flags));
			try!(hdr_cur.write_u64::<LittleEndian>(absgp));
			try!(hdr_cur.write_u32::<LittleEndian>(serial));
			try!(hdr_cur.write_u32::<LittleEndian>(pg.sequence_num));

			// checksum, calculated later on :)
			// Don't do excessive checking here (that the seek
			// succeeded & we are at the right pos now).
			// Its hopefully not required.
			try!(hdr_cur.seek(SeekFrom::Current(4)));

			try!(hdr_cur.write_u8(pg.segment_cnt));

			let mut hash_calculated :u32;

			let pg_lacing = &pg.cur_pg_lacing[0 .. pg.segment_cnt as usize];
			let pck_data = &pg.cur_pg_data;

			hash_calculated = vorbis_crc32_update(0, hdr_cur.get_ref());
			hash_calculated = vorbis_crc32_update(hash_calculated, pg_lacing);
			for (idx, pck) in pck_data.iter().enumerate() {
				let mut start :usize = 0;
				if idx == 0 { if let Some(idx) = pg.pck_last_overflow_idx {
					start = idx;
				}}
				let mut end :usize = pck.len();
				if idx + 1 == pck_data.len() {
					if let Some(idx) = pg.pck_this_overflow_idx {
						end = idx;
					}
				}
				hash_calculated = vorbis_crc32_update(hash_calculated,
					&pck[start .. end]);
			}

			// Go back to enter the checksum
			// Don't do excessive checking here (that the seek
			// succeeded & we are at the right pos now).
			// Its hopefully not required.
			try!(hdr_cur.seek(SeekFrom::Start(22)));
			try!(hdr_cur.write_u32::<LittleEndian>(hash_calculated));

			// Now all is done, write the stuff!
			try!(wtr.write_all(hdr_cur.get_ref()));
			try!(wtr.write_all(pg_lacing));
			for (idx, pck) in pck_data.iter().enumerate() {
				let mut start :usize = 0;
				if idx == 0 { if let Some(idx) = pg.pck_last_overflow_idx {
					start = idx;
				}}
				let mut end :usize = pck.len();
				if idx + 1 == pck_data.len() {
					if let Some(idx) = pg.pck_this_overflow_idx {
						end = idx;
					}
				}
				try!(wtr.write_all(&pck[start .. end]));
			}
		}

		// Reset the page.
		pg.first_page = false;
		pg.sequence_num += 1;

		pg.segment_cnt = 0;
		pg.cur_pg_data.clear();

		pg.pck_last_overflow_idx = pg.pck_this_overflow_idx;
		pg.pck_this_overflow_idx = None;

		return Ok(());
	}
}

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
		let mut r = PacketReader::new(&mut c);
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
		let mut r = PacketReader::new(&mut c);
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
		let mut r = PacketReader::new(&mut c);
		let p1 = r.read_packet().unwrap();
		assert_eq!(test_arr, *p1.data);
		let p2 = r.read_packet().unwrap();
		assert_eq!(test_arr_2, *p2.data);
		let p3 = r.read_packet().unwrap();
		assert_eq!(test_arr_3, *p3.data);
	}
}
