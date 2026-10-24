//! Hashing, checksums, a byte cursor and a fixed-point number.

use core::fmt;
use core::str::FromStr;

use crate::error::{KernelError, KernelResult};

/// The offset basis of FNV-1a over 64 bits.
pub const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
/// The prime multiply of FNV-1a over 64 bits.
pub const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// The FNV-1a hash of `bytes`.
///
/// The hash is cheap and independent of the allocator, which is what a table
/// key or a cache wants; it is not collision resistant and is not a credential.
pub const fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET_BASIS;
    let mut index = 0;
    while index < bytes.len() {
        hash ^= bytes[index] as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
        index += 1;
    }
    hash
}

/// The CRC-32 table, built at compile time.
const fn crc32_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut index = 0;
    while index < 256 {
        let mut value = index as u32;
        let mut bit = 0;
        while bit < 8 {
            value = if value & 1 == 0 {
                value >> 1
            } else {
                0xedb8_8320 ^ (value >> 1)
            };
            bit += 1;
        }
        table[index] = value;
        index += 1;
    }
    table
}

static CRC32_TABLE: [u32; 256] = crc32_table();

/// The CRC-32 checksum of `bytes`, as the ETag-style CRC uses it.
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in bytes {
        let index = ((crc ^ u32::from(*byte)) & 0xff) as usize;
        crc = CRC32_TABLE[index] ^ (crc >> 8);
    }
    crc ^ 0xffff_ffff
}

/// Writes `bytes` as lowercase hexadecimal.
pub fn hex_encode(bytes: &[u8]) -> String {
    const DIGITS: [u8; 16] = *b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}

/// Reads lowercase or uppercase hexadecimal.
pub fn hex_decode(text: &str) -> KernelResult<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return Err(KernelError::InvalidNumber);
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(text.len() / 2);
    let mut index = 0;
    while index < bytes.len() {
        let high = digit_value(bytes[index], index)?;
        let low = digit_value(bytes[index + 1], index + 1)?;
        out.push(high << 4 | low);
        index += 2;
    }
    Ok(out)
}

fn digit_value(byte: u8, offset: usize) -> KernelResult<u8> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(KernelError::UnexpectedByte { offset, byte }),
    }
}

/// A cursor over bytes that arrived from somewhere else.
///
/// Every read reports how it failed instead of panicking, which is what a parser
/// of untrusted input wants.
#[derive(Debug, Clone)]
pub struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    /// A cursor at the start of `bytes`.
    pub const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    /// How many bytes have been read.
    pub const fn offset(&self) -> usize {
        self.offset
    }

    /// The bytes that are left.
    pub fn remaining(&self) -> &'a [u8] {
        &self.bytes[self.offset..]
    }

    /// Reports whether every byte has been read.
    pub const fn is_empty(&self) -> bool {
        self.offset == self.bytes.len()
    }

    /// Reads one byte.
    pub fn byte(&mut self) -> KernelResult<u8> {
        let byte = *self.bytes.get(self.offset).ok_or(KernelError::UnexpectedByte {
            offset: self.offset,
            byte: 0,
        })?;
        self.offset += 1;
        Ok(byte)
    }

    /// Reads `count` bytes.
    pub fn take(&mut self, count: usize) -> KernelResult<&'a [u8]> {
        let end = self.offset.checked_add(count).ok_or(KernelError::Overflow)?;
        let slice = self
            .bytes
            .get(self.offset..end)
            .ok_or(KernelError::UnexpectedByte {
                offset: self.offset,
                byte: 0,
            })?;
        self.offset = end;
        Ok(slice)
    }

    /// Reads four bytes as a big-endian number.
    pub fn u32_be(&mut self) -> KernelResult<u32> {
        let bytes = self.take(4)?;
        Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }
}

/// A number with six decimal places, stored as millionths.
///
/// The representation is exact, so two values that were written the same way are
/// equal and a sum of them does not drift. Arithmetic reports overflow instead
/// of wrapping.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Fixed {
    micros: i64,
}

impl Fixed {
    /// The number of millionths in one unit.
    pub const SCALE: i64 = 1_000_000;

    /// The value `micros` millionths.
    pub const fn from_micros(micros: i64) -> Self {
        Self { micros }
    }

    /// The value in millionths.
    pub const fn micros(self) -> i64 {
        self.micros
    }

    /// The whole and fractional parts, each carrying the sign.
    pub const fn split(self) -> (i64, i64) {
        (self.micros / Self::SCALE, self.micros % Self::SCALE)
    }

    /// The sum, or `None` when it does not fit.
    pub const fn checked_add(self, other: Self) -> Option<Self> {
        match self.micros.checked_add(other.micros) {
            Some(micros) => Some(Self { micros }),
            None => None,
        }
    }

    /// The difference, or `None` when it does not fit.
    pub const fn checked_sub(self, other: Self) -> Option<Self> {
        match self.micros.checked_sub(other.micros) {
            Some(micros) => Some(Self { micros }),
            None => None,
        }
    }

    /// Reads a decimal number with at most six fractional digits.
    pub fn parse(text: &str) -> KernelResult<Self> {
        let bytes = text.as_bytes();
        if bytes.is_empty() {
            return Err(KernelError::InvalidNumber);
        }
        let mut index = 0;
        let negative = match bytes[0] {
            b'-' => {
                index = 1;
                true
            }
            b'+' => {
                index = 1;
                false
            }
            _ => false,
        };
        let mut whole: i64 = 0;
        let mut whole_digits = 0;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            whole = whole
                .checked_mul(10)
                .and_then(|value| value.checked_add(i64::from(bytes[index] - b'0')))
                .ok_or(KernelError::Overflow)?;
            index += 1;
            whole_digits += 1;
        }
        let mut fraction: i64 = 0;
        let mut fraction_digits = 0;
        if let Some(b'.') = bytes.get(index) {
            index += 1;
            while index < bytes.len() && bytes[index].is_ascii_digit() {
                if fraction_digits >= 6 {
                    return Err(KernelError::InvalidNumber);
                }
                fraction = fraction * 10 + i64::from(bytes[index] - b'0');
                fraction_digits += 1;
                index += 1;
            }
            if fraction_digits == 0 {
                return Err(KernelError::InvalidNumber);
            }
        }
        if whole_digits == 0 || index != bytes.len() {
            return Err(KernelError::InvalidNumber);
        }
        while fraction_digits < 6 {
            fraction *= 10;
            fraction_digits += 1;
        }
        let micros = whole
            .checked_mul(Self::SCALE)
            .and_then(|value| value.checked_add(fraction))
            .ok_or(KernelError::Overflow)?;
        Ok(Self::from_micros(if negative { -micros } else { micros }))
    }
}

impl fmt::Display for Fixed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sign = if self.micros < 0 { "-" } else { "" };
        let magnitude = self.micros.unsigned_abs();
        write!(
            formatter,
            "{sign}{}.{:06}",
            magnitude / Self::SCALE as u64,
            magnitude % Self::SCALE as u64
        )
    }
}

impl FromStr for Fixed {
    type Err = KernelError;

    fn from_str(text: &str) -> KernelResult<Self> {
        Self::parse(text)
    }
}

/// Something that reduces bytes to a number.
///
/// The associated type is what makes one trait serve a hash and a checksum: the
/// caller names the result it wants and the implementation decides.
pub trait Encode {
    /// The number the encoding produces.
    type Output;

    /// Encodes `bytes`.
    fn encode(&self, bytes: &[u8]) -> Self::Output;
}

/// The FNV-1a form of [`Encode`].
#[derive(Debug, Default)]
pub struct FnvEncode;

impl Encode for FnvEncode {
    type Output = u64;

    fn encode(&self, bytes: &[u8]) -> u64 {
        fnv1a(bytes)
    }
}

/// The CRC-32 form of [`Encode`].
#[derive(Debug, Default)]
pub struct Crc32Encode;

impl Encode for Crc32Encode {
    type Output = u32;

    fn encode(&self, bytes: &[u8]) -> u32 {
        crc32(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hash_matches_its_published_vectors() {
        assert_eq!(fnv1a(b""), FNV_OFFSET_BASIS);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a(b"hello"), 0xa430_d846_80aa_bd0b);
    }

    #[test]
    fn the_checksum_matches_its_published_vector() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
    }

    #[test]
    fn hexadecimal_round_trips_and_refuses_odd_input() {
        assert_eq!(hex_encode(&[0x00, 0x0f, 0xff]), "000fff");
        assert_eq!(hex_decode("000FfF").unwrap(), vec![0x00, 0x0f, 0xff]);
        assert_eq!(
            hex_decode("abc").unwrap_err(),
            KernelError::InvalidNumber
        );
        assert!(matches!(
            hex_decode("zz"),
            Err(KernelError::UnexpectedByte { offset: 0, .. })
        ));
    }

    #[test]
    fn the_cursor_reports_the_offset_it_stopped_at() {
        let mut cursor = Cursor::new(&[1, 2, 3, 4, 5]);
        assert_eq!(cursor.byte().unwrap(), 1);
        assert_eq!(cursor.u32_be().unwrap_or(0), 0x0203_0405);
        assert!(cursor.is_empty());
        assert!(cursor.byte().is_err());
        assert!(Cursor::new(&[1]).u32_be().is_err());
    }

    #[test]
    fn a_fixed_number_is_exact() {
        assert_eq!(Fixed::parse("12.3456").unwrap().micros(), 12_345_600);
        assert_eq!(Fixed::parse("-0.5").unwrap().micros(), -500_000);
        assert_eq!(Fixed::parse("7").unwrap().micros(), 7_000_000);
        assert_eq!(Fixed::parse("1.0000001").unwrap_err(), KernelError::InvalidNumber);
        assert_eq!(Fixed::parse("").unwrap_err(), KernelError::InvalidNumber);
        assert_eq!(Fixed::parse(".5").unwrap_err(), KernelError::InvalidNumber);
        assert_eq!(Fixed::parse("1.").unwrap_err(), KernelError::InvalidNumber);
        assert_eq!(Fixed::parse("1x").unwrap_err(), KernelError::InvalidNumber);
    }

    #[test]
    fn sums_and_differences_report_overflow() {
        let one = Fixed::from_micros(1_000_000);
        let two = Fixed::from_micros(2_500_000);
        assert_eq!(one.checked_add(two).unwrap().micros(), 3_500_000);
        assert_eq!(two.checked_sub(one).unwrap().micros(), 1_500_000);
        let huge = Fixed::from_micros(i64::MAX);
        assert_eq!(huge.checked_add(one), None);
        assert_eq!(Fixed::from_micros(1_500_000).to_string(), "1.500000");
        assert_eq!(Fixed::from_micros(-1_500_000).to_string(), "-1.500000");
    }

    #[test]
    fn the_encoding_trait_names_its_output() {
        let bytes = b"payload";
        let hash: u64 = FnvEncode.encode(bytes);
        let check: u32 = Crc32Encode.encode(bytes);
        assert_eq!(hash, fnv1a(bytes));
        assert_eq!(check, crc32(bytes));
    }
}
