// Ogg decoder and encoder written in Rust
//
// Copyright (c) 2016 est31 <MTest31@outlook.com>
// and contributors. All rights reserved.
// Redistribution or use only under the terms
// specified in the LICENSE file attached to this
// source distribution.

use std::io::{Read, Seek, Result};

/**
Buffering wrapper for async operation

This struct may be used together with activation of the `async`
feature to enable async operation of the `PacketReader` struct,
by implementing the `AdvanceAndSeekBack` trait.

It functions as wrapper to give this functinoality to an existing
`Read` implementation.

You may use your own implementation of that trait as well, but
for bug free operation you need to conform the rules the interface
documentation has set.
*/
pub struct BufReader<T :Read> {
	rdr :T,
	buf :Vec<u8>,
	read_offs :usize,
}

impl <T :Read> BufReader<T> {
	pub fn new(rdr :T) -> Self {
		return BufReader {
			rdr : rdr,
			buf : Vec::new(),
			read_offs : 0,
		};
	}
	pub fn into_inner(self) -> T {
		self.rdr
	}
}

/**
Interface for read failure recovery layers

This provides the protocol that can be used
together with the `Read` (and possibly `Seek`) traits
to manage recovery when read failures happen.

The recovery mechanism is as follows: From time to time,
users of this trait will call the `advance()` method.
Calling the method indicates that the state is safe.

Once a read failure (e.g. of the `WouldBlock` error
kind) happens, the implementor of this trait is expected
to detect this condition and revert the state to the
last time the `advance()` method has been called.

The user is expected to call `advance()` regularly to
prevent leaks from happening.

## Clean and "dirty" state

If the user calls `advance`, or uses a freshly constructed
implementation, the state is "clean", meaning that a failing
read call would yield in no change of state, because the
current state is already the one being reverted to.
The state gets "dirty" again if a successful read call is
executed. If the state is dirty, and a read invocation fails,
it stays dirty until the next time `advance` gets called.

## Seeking

If the implementor also implements the Seek trait,
the implementation may panic if its state is not
"clean", like defined above.

Users are expected to use the `seek_back` method
instead, which allows seeking backwards from the
current read position by a maximum amount of the
bytes that have been successfully read since the
last time the state was "clean".

Note that the `seek_back` function is not intended to be
called by the user when a read call fails to roll back the state.
The rollback is done by the implementor of the trait automatically.
*/
pub trait AdvanceAndSeekBack {
	fn advance(&mut self);
	fn seek_back(&mut self, len :usize);
}

impl <T :Read> AdvanceAndSeekBack for BufReader<T> {
	/// Drops everything
	fn advance(&mut self) {
		self.buf = self.buf.split_off(self.read_offs);
		self.read_offs = 0;
	}

	fn seek_back(&mut self, len :usize) {
		if len > self.read_offs {
			panic!("Seek back of length {} can't exceed the internal buffer of size {}!",
				len, self.read_offs);
		}
		self.read_offs -= len;
	}
}

impl <T :Read> Read for BufReader<T> {

	fn read(&mut self, buf: &mut[u8]) -> Result<usize> {
		if self.buf.len() - self.read_offs > 0 {
			// Fill buf as far as we can with content from self.buf
			let ml = ::std::cmp::min(self.buf.len() - self.read_offs, buf.len());
			&mut buf[0 .. ml].copy_from_slice(&self.buf[self.read_offs .. (ml + self.read_offs)]);
			self.read_offs += ml;
			return Ok(ml);
		}
		match self.rdr.read(buf) {
			Ok(len) => {
				self.buf.extend_from_slice(&buf[0 .. len]);
				self.read_offs += len;
				return Ok(len);
			},
			Err(e) => {
				self.read_offs = 0;
				return Err(e);
			},
		}
	}

}

use std::io::SeekFrom;
impl <T :Read + Seek> Seek for BufReader<T> {
	fn seek(&mut self, from :SeekFrom) -> Result<u64> {
		if self.read_offs > 0 || self.buf.len() > 0 {
			panic!("Seek invoked in non clean state!");
		}
		return self.rdr.seek(from);
	}
}
