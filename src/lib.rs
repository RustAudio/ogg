// Ogg decoder and encoder written in Rust
//
// Copyright (c) 2016 est31 <MTest31@outlook.com>
// and contributors. All rights reserved.
// Redistribution or use only under the terms
// specified in the LICENSE file attached to this
// source distribution.

#![allow(unknown_lints)]
#![forbid(unsafe_code)]
#![allow(clippy::needless_return)]
#![allow(clippy::nonminimal_bool)]
#![allow(clippy::tabs_in_doc_comments)]
#![allow(clippy::new_without_default)]
#![allow(clippy::partialeq_to_none)]

#![cfg_attr(docsrs, feature(doc_auto_cfg))]

/*!
Ogg container decoder and encoder

The most interesting structures for in this
mod are `PacketReader` and `PacketWriter`.
*/

#[cfg(test)]
mod test;

macro_rules! tri {
	($e:expr) => {
		match $e {
			Ok(val) => val,
			Err(err) => return Err(err.into()),
		}
	};
}

mod crc;
pub mod reading;
pub mod writing;

pub use crate::writing::{PacketWriter, PacketWriteEndInfo};
pub use crate::reading::{PacketReader, OggReadError};

/**
Ogg packet representation.

For the Ogg format, packets are the logically smallest subdivision it handles.

Every packet belongs to a *logical* bitstream. The *logical* bitstreams then form a *physical* bitstream, with the data combined in multiple different ways.

Every logical bitstream is identified by the serial number its pages have stored. The Packet struct contains a field for that number as well, so that one can find out which logical bitstream the Packet belongs to.
*/
pub struct Packet {
	/// The data the `Packet` contains
	pub data :Vec<u8>,
	/// `true` iff this packet is the first one in the page.
	first_packet_pg :bool,
	/// `true` iff this packet is the first one in the logical bitstream.
	first_packet_stream :bool,
	/// `true` iff this packet is the last one in the page.
	last_packet_pg :bool,
	/// `true` iff this packet is the last one in the logical bitstream
	last_packet_stream :bool,
	/// Absolute granule position of the last page the packet was in.
	/// The meaning of the absolute granule position is defined by the codec.
	absgp_page :u64,
	/// Serial number. Uniquely identifying the logical bitstream.
	stream_serial :u32,
	/// Checksum of the last page the packet was in.
	checksum_page :u32,
	/*/// Packet counter
	/// Why u64? There are MAX_U32 pages, and every page has up to 128 packets. u32 wouldn't be sufficient here...
	pub sequence_num :u64,*/ // TODO perhaps add this later on...
}

impl Packet {
	/// Returns whether the packet is the first one starting in the page
	pub fn first_in_page(&self) -> bool {
		self.first_packet_pg
	}
	/// Returns whether the packet is the first one of the entire stream
	pub fn first_in_stream(&self) -> bool {
		self.first_packet_stream
	}
	/// Returns whether the packet is the last one starting in the page
	pub fn last_in_page(&self) -> bool {
		self.last_packet_pg
	}
	/// Returns whether the packet is the last one of the entire stream
	pub fn last_in_stream(&self) -> bool {
		self.last_packet_stream
	}
	/// Returns the absolute granule position of the page the packet ended in.
	///
	/// The meaning of the absolute granule position is defined by the codec.
	pub fn absgp_page(&self) -> u64 {
		self.absgp_page
	}
	/// Returns the serial number that uniquely identifies the logical bitstream.
	pub fn stream_serial(&self) -> u32 {
		self.stream_serial
	}
	/// Returns the checksum of the page the packet ended in.
	pub fn checksum_page(&self) -> u32 {
		self.checksum_page
	}
}
