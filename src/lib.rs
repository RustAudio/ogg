// Ogg decoder and encoder written in Rust
//
// Copyright (c) 2016 est31 <MTest31@outlook.com>
// and contributors. All rights reserved.
// Redistribution or use only under the terms
// specified in the LICENSE file attached to this
// source distribution.

#![forbid(unsafe_code)]
#![cfg_attr(test, deny(warnings))]

/*!
Ogg container decoder and encoder

The most interesting structures for in this
mod are `PacketReader` and `PacketWriter`.
*/

extern crate byteorder;
#[cfg(feature = "async")]
extern crate tokio_io;
#[cfg(feature = "async")]
#[macro_use]
extern crate futures;
#[cfg(feature = "async")]
extern crate bytes;

#[cfg(test)]
mod test;

mod crc;
pub mod reading;
pub mod writing;

pub use writing::{PacketWriter, PacketWriteEndInfo};
pub use reading::{PacketReader, OggReadError};

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
