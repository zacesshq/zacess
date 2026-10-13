//! Base64 decoding for images sent inside a capture request. Both the standard and the URL-safe
//! alphabets are read, padding is optional and whitespace is skipped, since shells differ.

fn value(c: u8) -> Option<u32> {
    match c {
        b'A'..=b'Z' => Some((c - b'A') as u32),
        b'a'..=b'z' => Some((c - b'a') as u32 + 26),
        b'0'..=b'9' => Some((c - b'0') as u32 + 52),
        b'+' | b'-' => Some(62),
        b'/' | b'_' => Some(63),
        _ => None,
    }
}

/// Decodes base64. Returns `None` when the text is not base64.
pub fn decode(text: &str) -> Option<Vec<u8>> {
    // a data URL from a browser carries its type before the comma
    let text = match text.find(";base64,") {
        Some(at) if text.starts_with("data:") => &text[at + 8..],
        _ => text,
    };
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut buffer = 0u32;
    let mut bits = 0u32;
    let mut padding = false;
    for &c in text.as_bytes() {
        if c.is_ascii_whitespace() {
            continue;
        }
        if c == b'=' {
            padding = true;
            continue;
        }
        if padding {
            // nothing but padding may follow padding
            return None;
        }
        buffer = (buffer << 6) | value(c)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    // a single leftover character carries fewer than eight bits, which no encoder writes
    if bits >= 6 {
        return None;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::decode;

    #[test]
    fn decodes_standard_url_safe_and_unpadded_text() {
        assert_eq!(decode("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(decode("aGVsbG8").unwrap(), b"hello");
        assert_eq!(decode("aGVs\nbG8h").unwrap(), b"hello!");
        assert_eq!(decode("-_8=").unwrap(), [0xfb, 0xff]);
        assert_eq!(decode("+/8=").unwrap(), [0xfb, 0xff]);
        assert_eq!(decode("data:image/png;base64,aGVsbG8=").unwrap(), b"hello");
        assert_eq!(decode("").unwrap(), b"");
    }

    #[test]
    fn refuses_text_that_is_not_base64() {
        assert!(decode("not base64!").is_none());
        assert!(decode("aGVsbG8=x").is_none());
        assert!(decode("a").is_none());
    }
}
