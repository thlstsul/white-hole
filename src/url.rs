use std::{borrow::Cow, collections::VecDeque, slice, str::Utf8Error};

const KEEP_ENCODED_CHAR: &[u8] = b" \"#$%&'()*+,/:;<=>?@[\\]^`{|}";

/// Percent-decode the given string.
///
/// <https://url.spec.whatwg.org/#string-percent-decode>
///
/// See [`percent_decode`] regarding the return type.
#[inline]
pub fn percent_decode_str(input: &str) -> PercentDecode<'_> {
    percent_decode(input.as_bytes())
}

/// Percent-decode the given bytes.
///
/// <https://url.spec.whatwg.org/#percent-decode>
///
/// Any sequence of `%` followed by two hexadecimal digits is decoded.
/// The return type:
///
/// * Implements `Into<Cow<u8>>` borrowing `input` when it contains no percent-encoded sequence,
/// * Implements `Iterator<Item = u8>` and therefore has a `.collect::<Vec<u8>>()` method,
/// * Has `decode_utf8()` and `decode_utf8_lossy()` methods.
///
/// # Examples
///
/// ```
/// use percent_encoding::percent_decode;
///
/// assert_eq!(percent_decode(b"foo%20bar%3f").decode_utf8().unwrap(), "foo bar?");
/// ```
#[inline]
pub fn percent_decode(input: &[u8]) -> PercentDecode<'_> {
    PercentDecode {
        bytes: input.iter(),
        temp: VecDeque::with_capacity(2),
    }
}

/// The return type of [`percent_decode`].
#[derive(Clone, Debug)]
pub struct PercentDecode<'a> {
    bytes: slice::Iter<'a, u8>,
    temp: VecDeque<u8>,
}

fn after_percent_sign(iter: &mut slice::Iter<'_, u8>) -> Option<u8> {
    let mut cloned_iter = iter.clone();
    let h = char::from(*cloned_iter.next()?).to_digit(16)?;
    let l = char::from(*cloned_iter.next()?).to_digit(16)?;
    *iter = cloned_iter;
    Some(h as u8 * 0x10 + l as u8)
}

fn encode(byte: u8) -> Vec<u8> {
    let hex = format!("{:02X}", byte);
    hex.into_bytes()
}

impl<'a> Iterator for PercentDecode<'a> {
    type Item = u8;

    fn next(&mut self) -> Option<u8> {
        if !self.temp.is_empty() {
            return self.temp.pop_front();
        }

        self.bytes.next().map(|&byte| {
            if byte == b'%' {
                let decoded_byte = after_percent_sign(&mut self.bytes).unwrap_or(byte);
                if KEEP_ENCODED_CHAR.contains(&decoded_byte) {
                    self.temp.extend(encode(decoded_byte));
                    byte
                } else {
                    decoded_byte
                }
            } else {
                byte
            }
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let bytes = self.bytes.len();
        (bytes.div_ceil(3), Some(bytes))
    }
}

impl<'a> From<PercentDecode<'a>> for Cow<'a, [u8]> {
    fn from(iter: PercentDecode<'a>) -> Self {
        match iter.if_any() {
            Some(vec) => Cow::Owned(vec),
            None => Cow::Borrowed(iter.bytes.as_slice()),
        }
    }
}

impl<'a> PercentDecode<'a> {
    /// If the percent-decoding is different from the input, return it as a new bytes vector.
    fn if_any(&self) -> Option<Vec<u8>> {
        let mut bytes_iter = self.bytes.clone();
        while bytes_iter.any(|&b| b == b'%') {
            if let Some(decoded_byte) = after_percent_sign(&mut bytes_iter) {
                if KEEP_ENCODED_CHAR.contains(&decoded_byte) {
                    continue;
                }
                let initial_bytes = self.bytes.as_slice();
                let unchanged_bytes_len = initial_bytes.len() - bytes_iter.len() - 3;
                let mut decoded = initial_bytes[..unchanged_bytes_len].to_owned();
                decoded.push(decoded_byte);
                decoded.extend(PercentDecode {
                    bytes: bytes_iter,
                    temp: VecDeque::with_capacity(2),
                });
                return Some(decoded);
            }
        }
        // Nothing to decode
        None
    }

    /// Decode the result of percent-decoding as UTF-8.
    ///
    /// This is return `Err` when the percent-decoded bytes are not well-formed in UTF-8.
    pub fn decode_utf8(self) -> Result<Cow<'a, str>, Utf8Error> {
        match self.clone().into() {
            Cow::Borrowed(bytes) => match std::str::from_utf8(bytes) {
                Ok(s) => Ok(s.into()),
                Err(e) => Err(e),
            },
            Cow::Owned(bytes) => match String::from_utf8(bytes) {
                Ok(s) => Ok(s.into()),
                Err(e) => Err(e.utf8_error()),
            },
        }
    }

    /// Decode the result of percent-decoding as UTF-8, lossily.
    ///
    /// Invalid UTF-8 percent-encoded byte sequences will be replaced � U+FFFD,
    /// the replacement character.
    pub fn decode_utf8_lossy(self) -> Cow<'a, str> {
        decode_utf8_lossy(self.clone().into())
    }
}

fn decode_utf8_lossy(input: Cow<'_, [u8]>) -> Cow<'_, str> {
    // Note: This function is duplicated in `form_urlencoded/src/query_encoding.rs`.
    match input {
        Cow::Borrowed(bytes) => String::from_utf8_lossy(bytes),
        Cow::Owned(bytes) => {
            match String::from_utf8_lossy(&bytes) {
                Cow::Borrowed(utf8) => {
                    // If from_utf8_lossy returns a Cow::Borrowed, then we can
                    // be sure our original bytes were valid UTF-8. This is because
                    // if the bytes were invalid UTF-8 from_utf8_lossy would have
                    // to allocate a new owned string to back the Cow so it could
                    // replace invalid bytes with a placeholder.

                    // First we do a debug_assert to confirm our description above.
                    let raw_utf8: *const [u8] = utf8.as_bytes();
                    debug_assert!(std::ptr::addr_eq(raw_utf8, &*bytes as *const [u8]));

                    // Given we know the original input bytes are valid UTF-8,
                    // and we have ownership of those bytes, we re-use them and
                    // return a Cow::Owned here.
                    Cow::Owned(unsafe { String::from_utf8_unchecked(bytes) })
                }
                Cow::Owned(s) => Cow::Owned(s),
            }
        }
    }
}

#[test]
fn test() {
    let url = "https://example.com/path/%E4%B8%AD%E6%96%87/search?q=%23Rust%2F编程%20%26%20解码!";
    let decoded = percent_decode_str(url).decode_utf8_lossy();
    println!("解码 URL: {}", decoded);
    assert_eq!(
        decoded,
        "https://example.com/path/中文/search?q=%23Rust%2F编程%20%26%20解码!"
    );
}
