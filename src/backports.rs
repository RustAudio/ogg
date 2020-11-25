// Copied from standback https://github.com/jhpratt/standback

pub trait Sealed<T: ?Sized> {}
impl<T: ?Sized> Sealed<T> for T {}

#[allow(non_camel_case_types)]
pub trait Option_v1_35<'a, T: Copy + 'a>: Sealed<Option<&'a T>> {
	fn copied(self) -> Option<T>;
}

impl<'a, T: Copy + 'a> Option_v1_35<'a, T> for Option<&'a T> {
	fn copied(self) -> Option<T> {
		self.map(|&t| t)
	}
}

#[allow(non_camel_case_types)]
pub trait u32_v1_32: Sealed<u32> {
	fn to_le_bytes(self) -> [u8; 4];
}

impl u32_v1_32 for u32 {
	fn to_le_bytes(self) -> [u8; 4] {
		// TODO: better implementation?
		[
			(self & 0xff) as u8,
			((self >> 8) & 0xff) as u8,
			((self >> 16) & 0xff) as u8,
			((self >> 24) & 0xff) as u8,
		]
	}
}
