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
use crc::vorbis_crc32_update;

#[cfg(feature = "async")]
mod buf_reader;

mod crc;

#[cfg(feature = "async")]
pub use buf_reader::BufReader as BufReader;
#[cfg(feature = "async")]
pub use buf_reader::AdvanceAndSeekBack as AdvanceAndSeekBack;

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

/// Containing information about an OGG page that is shared between multiple places
struct PageBaseInfo {
	/// `true`: the first packet is continued from the page before. `false`: if its a "fresh" one
	starts_with_continued :bool,
	/// `true` if this page is the first one in the logical bitstream
	first_page :bool,
	/// `true` if this page is the last one in the logical bitstream
	last_page :bool,
	/// Absolute granule position. The codec defines further meaning.
	absgp :u64,
	/// Page counter
	#[allow(unused)]
	sequence_num :u32,
	/// Packet information:
	/// index is number of packet,
	/// tuple is (offset, length) of packet
	/// if ends_with_continued is true, the last element will contain information
	/// about the continued packet
	packet_positions :Vec<(u16,u16)>,
	/// `true` if the packet is continued in subsequent page(s)
	/// `false` if the packet has a segment of length < 255 inside this page
	ends_with_continued :bool,
}

/// Internal helper struct for PacketReader state
struct PageInfo {
	/// Basic information about the last read page
	bi :PageBaseInfo,
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
		return ((self.packet_idx + 1 + (self.bi.ends_with_continued as u8)) as usize
			== self.bi.packet_positions.len());
	}
}

/**
Helper struct for parsing pages

Its created using the `new` function and then its fed more data via the `parse_segments`
and `parse_packet_data` functions, each called exactly once and in that precise order.

Then later code uses the struct's contents.
*/
struct PageParser {
	// Members packet_positions and packet_count
	// get populated after segments have been parsed
	bi :PageBaseInfo,

	stream_serial :u32,
	checksum :u32,
	header_buf: [u8; 27],
	/// Number of packet ending segments
	packet_count :u16, // Gets populated gafter segments have been parsed
	/// after segments have been parsed, this contains the segments buffer,
	/// after the packet data have been read, this contains the packets buffer.
	segments_or_packets_buf :Vec<u8>,
}

impl PageParser {
	/// Creates a new Page parser
	///
	/// The `header_buf` param contains the first 27 bytes of a new OGG page.
	/// Determining when one begins is your responsibility. Usually they
	/// begin directly after the end of a previous OGG page, but
	/// after you've performed a seek you might end up within the middle of a page
	/// and need to recapture.
	///
	/// Returns a page parser, and the requested size of the segments array.
	/// You should allocate and fill such an array, in order to pass it to the `parse_segments`
	/// function.
	fn new(header_buf :[u8; 27]) -> Result<(PageParser, usize), OggReadError> {
		let mut header_rdr = Cursor::new(header_buf);
		header_rdr.set_position(4);
		let stream_structure_version = try!(header_rdr.read_u8());
		if stream_structure_version != 0 {
			try!(Err(OggReadError::InvalidStreamStructVer(stream_structure_version)));
		}
		let header_type_flag = header_rdr.read_u8().unwrap();
		let stream_serial;

		Ok((PageParser {
			bi : PageBaseInfo {
				starts_with_continued : header_type_flag & 0x01u8 != 0,
				first_page : header_type_flag & 0x02u8 != 0,
				last_page : header_type_flag & 0x04u8 != 0,
				absgp : header_rdr.read_u64::<LittleEndian>().unwrap(),
				sequence_num : {
					stream_serial = header_rdr.read_u32::<LittleEndian>().unwrap();
					header_rdr.read_u32::<LittleEndian>().unwrap()
				},
				packet_positions : Vec::new(),
				ends_with_continued : false,
			},
			stream_serial,
			checksum : header_rdr.read_u32::<LittleEndian>().unwrap(),
			header_buf : header_buf,
			packet_count : 0,
			segments_or_packets_buf :Vec::new(),
		},
			// Number of page segments
			header_rdr.read_u8().unwrap() as usize
		))
	}

	/// Parses the segments buffer, and returns the requested size
	/// of the packets content array.
	fn parse_segments(&mut self, segments_buf :Vec<u8>) -> usize {
		let mut page_siz :u16 = 0; // Size of the page's body
		// Whether our page ends with a continued packet
		self.bi.ends_with_continued = self.bi.starts_with_continued;

		// First run: get the number of packets,
		// whether the page ends with a continued packet,
		// and the size of the page's body
		for val in &segments_buf {
			page_siz += *val as u16;
			// Increment by 1 if val < 255, otherwise by 0
			self.packet_count += (*val < 255) as u16;
			self.bi.ends_with_continued = !(*val < 255);
		}

		let mut packets = Vec::with_capacity(self.packet_count as usize
			+ self.bi.ends_with_continued as usize);
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
		if self.bi.ends_with_continued {
			packets.push((cur_packet_offs, cur_packet_siz));
		}

		self.bi.packet_positions = packets;
		self.segments_or_packets_buf = segments_buf;
		page_siz as usize
	}

	/// Parses the packets data and verifies the checksum.
	///
	/// Only after this function has been called (and before it `parse_segments`)
	/// you should pass on the `PageParser` to later code.
	fn parse_packet_data(&mut self, packet_data :Vec<u8>) -> Result<(), OggReadError> {
		// Now to hash calculation.
		// 1. Clear the header buffer
		self.header_buf[22] = 0;
		self.header_buf[23] = 0;
		self.header_buf[24] = 0;
		self.header_buf[25] = 0;

		// 2. Calculate the hash
		let mut hash_calculated :u32;
		hash_calculated = vorbis_crc32_update(0, &self.header_buf);
		hash_calculated = vorbis_crc32_update(hash_calculated,
			&self.segments_or_packets_buf);
		hash_calculated = vorbis_crc32_update(hash_calculated, &packet_data);

		// 3. Compare to the extracted one
		if self.checksum != hash_calculated {
			try!(Err(OggReadError::HashMismatch(self.checksum, hash_calculated)));
		}
		self.segments_or_packets_buf = packet_data;
		Ok(())
	}
}

/**
Reader for packets from an Ogg stream.

This reads codec packets belonging to several different logical streams from one physical Ogg container stream.

If the `async` feature is activated, and you pass as internal reader a valid implementation of the
`AdvanceAndSeekBack` trait, like the `BufReader` wrapper, the PacketReader will support async operation,
meaning that its internal state doesn't get corrupted if from multiple consecutive reads which it performs,
some fail with e.g. the `WouldBlock` error kind.
*/
pub struct PacketReader<T :io::Read + io::Seek> {
	rdr :T,

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

impl<T :io::Read + io::Seek> PacketReader <T> {
	/// Constructs a new `PacketReader` with a given `Read`.
	pub fn new(rdr :T) -> PacketReader<T> {
		return PacketReader { rdr: rdr, page_infos: HashMap::new(),
			stream_with_stuff: None, has_seeked: false };
	}
	pub fn into_inner(self) -> T {
		self.rdr
	}
	/// Reads a packet, and returns it on success.
	pub fn read_packet(&mut self) -> Result<Packet, OggReadError> {
		while self.stream_with_stuff == None {
			let page = try!(self.read_ogg_page());
			try!(self.push_page(page));
		}
		let str_serial :u32 = self.stream_with_stuff.unwrap();
		let mut pg_info = self.page_infos.get_mut(&str_serial).unwrap();
		let (offs, len) = pg_info.bi.packet_positions[pg_info.packet_idx as usize];
		// If there is a continued packet, and we are at the start right now,
		// and we actually have its end in the current page, glue it together.
		let packet_content :Vec<u8> = if (
				pg_info.packet_idx == 0 && pg_info.bi.starts_with_continued
				&& !(pg_info.bi.ends_with_continued && pg_info.bi.packet_positions.len() == 1)
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
		let first_pck_overall = pg_info.bi.first_page && first_pck_in_pg;

		let last_pck_in_pg = pg_info.is_last_pck_in_pg();
		let last_pck_overall = pg_info.bi.last_page && last_pck_in_pg;

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
			absgp_page: pg_info.bi.absgp,
			stream_serial: str_serial,
		});
	}

	/// Reads until the new page header, and then returns the page header array.
	///
	/// If no new page header is immediately found, it performs a "recapture",
	/// meaning it searches for the capture pattern, and if it finds it, it
	/// reads the complete first 27 bytes of the header, and returns them.
	fn read_until_pg_header(&mut self) -> Result<[u8; 27], OggReadError> {
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

	/// Parses and reads a new OGG page
	///
	/// Returns a fully complete `PageParser`
	///
	/// To support seeking this does not assume that the capture pattern
	/// is at the current reader position.
	/// Instead it searches until it finds the capture pattern.
	fn read_ogg_page(&mut self) -> Result<PageParser, OggReadError> {
		let header_buf :[u8; 27] = try!(self.read_until_pg_header());
		let (mut pg_prs, page_segments) = try!(PageParser::new(header_buf));

		let mut segments_buf = vec![0; page_segments]; // TODO fix this, we initialize memory for NOTHING!!! Out of some reason, this is seen as "unsafe" by rustc.
		try!(self.rdr.read_exact(&mut segments_buf));

		let page_siz = pg_prs.parse_segments(segments_buf);

		let mut packet_data = vec![0; page_siz as usize];
		try!(self.rdr.read_exact(&mut packet_data));

		self.maybe_advance();

		try!(pg_prs.parse_packet_data(packet_data));
		Ok(pg_prs)
	}

	/// Pushes a given OGG page, updating the internal structures
	/// with its contents.
	fn push_page(&mut self, mut pg_prs :PageParser) -> Result<(), OggReadError> {

		match self.page_infos.entry(pg_prs.stream_serial) {
			Entry::Occupied(mut o) => {
				let inf = o.get_mut();
				if pg_prs.bi.first_page {
					try!(Err(OggReadError::InvalidData));
				}
				if pg_prs.bi.starts_with_continued != inf.bi.ends_with_continued {
					if !self.has_seeked {
						try!(Err(OggReadError::InvalidData));
					} else {
						// If we have seeked, we are more tolerant here,
						// and just drop the continued packet's content.

						inf.last_overlap_pck.clear();
						if pg_prs.bi.starts_with_continued {
							pg_prs.bi.packet_positions.remove(0);
							if pg_prs.packet_count != 0 {
								// Decrease packet count by one. Normal case.
								pg_prs.packet_count -= 1;
							} else {
								// If the packet count is 0, this means
								// that we start and end with the same continued packet.
								// So now as we ignore that packet, we must clear the
								// ends_with_continued state as well.
								pg_prs.bi.ends_with_continued = false;
							}
						}
					}
				} else if pg_prs.bi.starts_with_continued {
					// Remember the packet at the end so that it can be glued together once
					// we encounter the next segment with length < 255 (doesnt have to be in this page)
					let (offs, len) = inf.bi.packet_positions[inf.packet_idx as usize];
					if len as usize != inf.page_body.len() {
						let mut tmp = Vec::with_capacity(len as usize);
						tmp.write_all(&inf.page_body[offs as usize .. (offs + len) as usize]).unwrap();
						inf.last_overlap_pck.push(tmp);
					} else {
						// Little optimisation: don't copy if not neccessary
						inf.last_overlap_pck.push(replace(&mut inf.page_body, vec![0;0]));
					}

				}
				inf.bi = pg_prs.bi;
				inf.packet_idx = 0;
				inf.page_body = pg_prs.segments_or_packets_buf;
			},
			Entry::Vacant(v) => {
				if !self.has_seeked {
					if (!pg_prs.bi.first_page || pg_prs.bi.starts_with_continued) {
						// If we haven't seeked, this is an error.
						try!(Err(OggReadError::InvalidData));
					}
				} else {
					if !pg_prs.bi.first_page {
						// we can just ignore this.
					}
					if pg_prs.bi.starts_with_continued {
						// Ignore the continued packet's content.
						// This is a normal occurence if we have just seeked.
						pg_prs.bi.packet_positions.remove(0);
						if pg_prs.packet_count != 0 {
							// Decrease packet count by one. Normal case.
							pg_prs.packet_count -= 1;
						} else {
							// If the packet count is 0, this means
							// that we start and end with the same continued packet.
							// So now as we ignore that packet, we must clear the
							// ends_with_continued state as well.
							pg_prs.bi.ends_with_continued = false;
						}
					}
				}
				v.insert(PageInfo {
					bi : pg_prs.bi,
					packet_idx: 0,
					page_body: pg_prs.segments_or_packets_buf,
					last_overlap_pck: Vec::new(),
				});
			},
		}
		let pg_has_stuff :bool = pg_prs.packet_count > 0;

		if pg_has_stuff {
			self.stream_with_stuff = Some(pg_prs.stream_serial);
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
				self.seek_back(len);
				return Ok(());
			}
		}
		return self.rdr.seek_back(len);
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
pub struct PacketWriter<T :io::Write> {
	wtr :T,

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

impl <T :io::Write> PacketWriter<T> {
	pub fn new(wtr :T) -> Self {
		return PacketWriter {
			wtr : wtr,
			page_vals : HashMap::new(),
		};
	}
	pub fn into_inner(self) -> T {
		self.wtr
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
					try!(PacketWriter::write_page(&mut self.wtr, serial, pg,
						false, absgp));
				} else {
					// We have to write a page end, and its the very last in the stream
					try!(PacketWriter::write_page(&mut self.wtr,
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
			try!(PacketWriter::write_page(&mut self.wtr, serial, pg,
				is_end_stream, absgp));

			pg.pck_last_overflow_idx = None;

			// TODO if inf was PacketWriteEndInfo::EndStream, we have to
			// somehow erase pg from the hashmap...
			// any ideas? perhaps needs external scope...
		}
		// All went fine.
		Ok(())
	}
	fn write_page(wtr :&mut T, serial :u32, pg :&mut CurrentPageValues,
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
