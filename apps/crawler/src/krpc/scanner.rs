
#[derive(Debug, Default)]
pub struct KrpcHeader<'a> {
    pub t: Option<&'a [u8]>,
    pub y: Option<&'a [u8]>,
    pub q: Option<&'a [u8]>,
}

pub fn scan(input: &[u8]) -> Option<KrpcHeader<'_>> {
    if input.is_empty() || input[0] != b'd' {
        return None;
    }
    let mut header = KrpcHeader::default();
    let mut pos = 1;

    while pos < input.len() {
        if input[pos] == b'e' {
            break;
        }

        // Parse key
        let key = parse_string(input, &mut pos)?;

        // Match key and parse value or skip
        if key == b"t" {
            header.t = parse_string(input, &mut pos);
        } else if key == b"y" {
            header.y = parse_string(input, &mut pos);
        } else if key == b"q" {
            header.q = parse_string(input, &mut pos);
        } else {
            // Skip value
            if !skip_value(input, &mut pos, 1) {
                return None;
            }
        }
    }
    Some(header)
}

fn parse_string<'a>(input: &'a [u8], pos: &mut usize) -> Option<&'a [u8]> {
    let mut colon = *pos;
    while colon < input.len() && input[colon] != b':' {
        if !input[colon].is_ascii_digit() {
            return None;
        }
        colon += 1;
    }
    if colon == input.len() || colon == *pos {
        return None;
    }

    let len_str = std::str::from_utf8(&input[*pos..colon]).ok()?;
    let len: usize = len_str.parse().ok()?;

    let start = colon + 1;
    let end = start + len;
    if end > input.len() {
        return None;
    }

    *pos = end;
    Some(&input[start..end])
}

fn skip_value(input: &[u8], pos: &mut usize, depth: usize) -> bool {
    if depth > 64 {
        return false;
    }
    if *pos >= input.len() {
        return false;
    }
    match input[*pos] {
        b'i' => {
            *pos += 1;
            while *pos < input.len() && input[*pos] != b'e' {
                *pos += 1;
            }
            if *pos < input.len() {
                *pos += 1; // consume 'e'
                true
            } else {
                false
            }
        }
        b'l' | b'd' => {
            *pos += 1;
            while *pos < input.len() && input[*pos] != b'e' {
                if !skip_value(input, pos, depth + 1) {
                    return false;
                }
            }
            if *pos < input.len() {
                *pos += 1; // consume 'e'
                true
            } else {
                false
            }
        }
        b'0'..=b'9' => parse_string(input, pos).is_some(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scan() {
        let krpc = b"d1:ad2:id20:abcdefghij0123456789e1:q9:find_node1:t2:aa1:y1:qe";
        let header = scan(krpc).unwrap();
        assert_eq!(header.t, Some(b"aa".as_slice()));
        assert_eq!(header.y, Some(b"q".as_slice()));
        assert_eq!(header.q, Some(b"find_node".as_slice()));
    }
}
