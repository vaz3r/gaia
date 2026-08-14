use std::collections::BTreeMap;

use gaia_bencode::{BencodeValue, from_bytes, to_bytes};
use pretty_assertions::assert_eq;
use serde::{Deserialize, Serialize};

// ============================================================
// Round-trip tests
// ============================================================

#[test]
fn round_trip_integer() {
    let val: i64 = 42;
    let encoded = to_bytes(&val).unwrap();
    assert_eq!(encoded, b"i42e");
    assert_eq!(from_bytes::<i64>(&encoded).unwrap(), val);
}

#[test]
fn round_trip_zero() {
    let val: i64 = 0;
    let encoded = to_bytes(&val).unwrap();
    assert_eq!(encoded, b"i0e");
    assert_eq!(from_bytes::<i64>(&encoded).unwrap(), val);
}

#[test]
fn round_trip_negative() {
    let val: i64 = -42;
    let encoded = to_bytes(&val).unwrap();
    assert_eq!(encoded, b"i-42e");
    assert_eq!(from_bytes::<i64>(&encoded).unwrap(), val);
}

#[test]
fn round_trip_large_integer() {
    let val: i64 = i64::MAX;
    let encoded = to_bytes(&val).unwrap();
    assert_eq!(from_bytes::<i64>(&encoded).unwrap(), val);
}

#[test]
fn round_trip_string() {
    let val = String::from("hello world");
    let encoded = to_bytes(&val).unwrap();
    assert_eq!(encoded, b"11:hello world");
    assert_eq!(from_bytes::<String>(&encoded).unwrap(), val);
}

#[test]
fn round_trip_empty_string() {
    let val = String::new();
    let encoded = to_bytes(&val).unwrap();
    assert_eq!(encoded, b"0:");
    assert_eq!(from_bytes::<String>(&encoded).unwrap(), val);
}

#[test]
fn round_trip_list_of_ints() {
    let val = vec![1i64, 2, 3];
    let encoded = to_bytes(&val).unwrap();
    assert_eq!(encoded, b"li1ei2ei3ee");
    assert_eq!(from_bytes::<Vec<i64>>(&encoded).unwrap(), val);
}

#[test]
fn round_trip_list_of_strings() {
    let val = vec!["spam".to_string(), "eggs".to_string()];
    let encoded = to_bytes(&val).unwrap();
    assert_eq!(encoded, b"l4:spam4:eggse");
    assert_eq!(from_bytes::<Vec<String>>(&encoded).unwrap(), val);
}

#[test]
fn round_trip_empty_list() {
    let val: Vec<i64> = vec![];
    let encoded = to_bytes(&val).unwrap();
    assert_eq!(encoded, b"le");
    assert_eq!(from_bytes::<Vec<i64>>(&encoded).unwrap(), val);
}

#[test]
fn round_trip_struct() {
    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct Info {
        length: i64,
        name: String,
        #[serde(rename = "piece length")]
        piece_length: i64,
    }

    let val = Info {
        length: 1_048_576,
        name: "test.txt".into(),
        piece_length: 262_144,
    };

    let encoded = to_bytes(&val).unwrap();
    let decoded: Info = from_bytes(&encoded).unwrap();
    assert_eq!(val, decoded);

    // Verify key ordering in encoded form
    let as_value: BencodeValue = from_bytes(&encoded).unwrap();
    if let BencodeValue::Dict(map) = &as_value {
        let keys: Vec<&[u8]> = map.keys().map(std::vec::Vec::as_slice).collect();
        assert_eq!(
            keys,
            vec![
                b"length".as_slice(),
                b"name".as_slice(),
                b"piece length".as_slice()
            ]
        );
    } else {
        panic!("expected dict");
    }
}

#[test]
fn round_trip_nested_struct() {
    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct Torrent {
        announce: String,
        info: Info,
    }

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct Info {
        length: i64,
        name: String,
    }

    let val = Torrent {
        announce: "http://tracker.example.com/announce".into(),
        info: Info {
            length: 1024,
            name: "file.dat".into(),
        },
    };

    let encoded = to_bytes(&val).unwrap();
    let decoded: Torrent = from_bytes(&encoded).unwrap();
    assert_eq!(val, decoded);
}

#[test]
fn round_trip_bencode_value() {
    let val = BencodeValue::Dict({
        let mut m = BTreeMap::new();
        m.insert(b"key".to_vec(), BencodeValue::Integer(42));
        m.insert(
            b"list".to_vec(),
            BencodeValue::List(vec![
                BencodeValue::Bytes(b"hello".to_vec()),
                BencodeValue::Integer(-1),
            ]),
        );
        m
    });

    let encoded = to_bytes(&val).unwrap();
    let decoded: BencodeValue = from_bytes(&encoded).unwrap();
    assert_eq!(val, decoded);
}

#[test]
fn round_trip_bytes_with_serde_bytes() {
    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct WithBytes {
        #[serde(with = "serde_bytes")]
        data: Vec<u8>,
    }

    let val = WithBytes {
        data: vec![0x00, 0xFF, 0xAA, 0x55],
    };

    let encoded = to_bytes(&val).unwrap();
    let decoded: WithBytes = from_bytes(&encoded).unwrap();
    assert_eq!(val, decoded);
}

// ============================================================
// BEP 3 spec compliance
// ============================================================

#[test]
fn bep3_integer_examples() {
    // From BEP 3: i3e represents 3
    assert_eq!(from_bytes::<i64>(b"i3e").unwrap(), 3);
    // i-3e represents -3
    assert_eq!(from_bytes::<i64>(b"i-3e").unwrap(), -3);
    // i0e represents 0
    assert_eq!(from_bytes::<i64>(b"i0e").unwrap(), 0);
}

#[test]
fn bep3_string_example() {
    // From BEP 3: 4:spam represents "spam"
    assert_eq!(from_bytes::<String>(b"4:spam").unwrap(), "spam");
}

#[test]
fn bep3_list_example() {
    // From BEP 3: l4:spam4:eggse represents ["spam", "eggs"]
    let val: Vec<String> = from_bytes(b"l4:spam4:eggse").unwrap();
    assert_eq!(val, vec!["spam", "eggs"]);
}

#[test]
fn bep3_dict_example() {
    // From BEP 3: d3:cow3:moo4:spam4:eggse represents {"cow": "moo", "spam": "eggs"}
    #[derive(Debug, Deserialize, PartialEq)]
    struct Example {
        cow: String,
        spam: String,
    }
    let val: Example = from_bytes(b"d3:cow3:moo4:spam4:eggse").unwrap();
    assert_eq!(
        val,
        Example {
            cow: "moo".into(),
            spam: "eggs".into(),
        }
    );
}

#[test]
fn bep3_dict_with_list() {
    // d4:spaml1:a1:bee represents {"spam": ["a", "b"]}
    #[derive(Debug, Deserialize, PartialEq)]
    struct Example {
        spam: Vec<String>,
    }
    let val: Example = from_bytes(b"d4:spaml1:a1:bee").unwrap();
    assert_eq!(
        val,
        Example {
            spam: vec!["a".into(), "b".into()],
        }
    );
}

// ============================================================
// Edge cases and error handling
// ============================================================

#[test]
fn reject_negative_zero() {
    assert!(from_bytes::<i64>(b"i-0e").is_err());
}

#[test]
fn reject_leading_zero() {
    assert!(from_bytes::<i64>(b"i03e").is_err());
    assert!(from_bytes::<i64>(b"i00e").is_err());
}

#[test]
fn reject_leading_zero_negative() {
    assert!(from_bytes::<i64>(b"i-03e").is_err());
}

#[test]
fn reject_empty_integer() {
    assert!(from_bytes::<i64>(b"ie").is_err());
}

#[test]
fn reject_bare_minus() {
    assert!(from_bytes::<i64>(b"i-e").is_err());
}

#[test]
fn reject_truncated_string() {
    assert!(from_bytes::<String>(b"5:abc").is_err());
}

#[test]
fn reject_truncated_integer() {
    assert!(from_bytes::<i64>(b"i42").is_err());
}

#[test]
fn reject_trailing_data() {
    assert!(from_bytes::<i64>(b"i42eXXX").is_err());
}

#[test]
fn reject_empty_input() {
    assert!(from_bytes::<i64>(b"").is_err());
}

#[test]
fn reject_invalid_start_byte() {
    assert!(from_bytes::<i64>(b"x42e").is_err());
}

#[test]
fn deeply_nested_lists() {
    // l l l i1e e e e
    let encoded = b"llli1eeee";
    let val: BencodeValue = from_bytes(encoded).unwrap();
    assert_eq!(
        val,
        BencodeValue::List(vec![BencodeValue::List(vec![BencodeValue::List(vec![
            BencodeValue::Integer(1)
        ])])])
    );

    // Round-trip
    let re_encoded = to_bytes(&val).unwrap();
    assert_eq!(re_encoded, encoded);
}

#[test]
fn empty_dict() {
    let encoded = b"de";
    let val: BencodeValue = from_bytes(encoded).unwrap();
    assert_eq!(val, BencodeValue::Dict(BTreeMap::new()));
    assert_eq!(to_bytes(&val).unwrap(), encoded.to_vec());
}

#[test]
fn empty_list() {
    let encoded = b"le";
    let val: BencodeValue = from_bytes(encoded).unwrap();
    assert_eq!(val, BencodeValue::List(vec![]));
}

#[test]
fn binary_string_data() {
    // Byte strings can contain arbitrary binary data
    let data = vec![0u8, 1, 2, 255, 254, 253];
    let encoded = to_bytes(&serde_bytes::ByteBuf::from(data.clone())).unwrap();
    let decoded: serde_bytes::ByteBuf = from_bytes(&encoded).unwrap();
    assert_eq!(decoded.as_ref(), &data);
}

#[test]
fn optional_field() {
    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct WithOptional {
        required: String,
        optional: Option<i64>,
    }

    let with = WithOptional {
        required: "yes".into(),
        optional: Some(42),
    };
    let encoded = to_bytes(&with).unwrap();
    let decoded: WithOptional = from_bytes(&encoded).unwrap();
    assert_eq!(with, decoded);
}

#[test]
fn unknown_fields_ignored() {
    // Deserializing should skip unknown keys in dicts
    #[derive(Debug, Deserialize, PartialEq)]
    struct Minimal {
        name: String,
    }

    let data = b"d5:extrai99e4:name4:teste";
    let val: Minimal = from_bytes(data).unwrap();
    assert_eq!(
        val,
        Minimal {
            name: "test".into()
        }
    );
}

// ============================================================
// BencodeValue display
// ============================================================

#[test]
fn display_integer() {
    assert_eq!(format!("{}", BencodeValue::Integer(42)), "42");
}

#[test]
fn display_string() {
    assert_eq!(
        format!("{}", BencodeValue::Bytes(b"hello".to_vec())),
        "\"hello\""
    );
}

#[test]
fn display_binary() {
    assert_eq!(
        format!("{}", BencodeValue::Bytes(vec![0xFF, 0xFE])),
        "<2 bytes>"
    );
}

#[test]
fn display_list() {
    let val = BencodeValue::List(vec![
        BencodeValue::Integer(1),
        BencodeValue::Bytes(b"two".to_vec()),
    ]);
    assert_eq!(format!("{val}"), "[1, \"two\"]");
}

#[test]
fn display_dict() {
    let mut map = BTreeMap::new();
    map.insert(b"key".to_vec(), BencodeValue::Integer(42));
    let val = BencodeValue::Dict(map);
    assert_eq!(format!("{val}"), "{\"key\": 42}");
}

// ============================================================
// Enum serialization
// ============================================================

#[test]
fn round_trip_unit_enum() {
    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    enum Color {
        Red,
        Green,
        Blue,
    }

    let encoded = to_bytes(&Color::Green).unwrap();
    assert_eq!(encoded, b"5:Green");
    assert_eq!(from_bytes::<Color>(&encoded).unwrap(), Color::Green);
}

// ============================================================
// Boolean support
// ============================================================

#[test]
fn round_trip_bool() {
    assert_eq!(to_bytes(&true).unwrap(), b"i1e");
    assert_eq!(to_bytes(&false).unwrap(), b"i0e");
    assert_eq!(from_bytes::<bool>(b"i1e").unwrap(), true);
    assert_eq!(from_bytes::<bool>(b"i0e").unwrap(), false);
}

// ============================================================
// Unsorted keys detection
// ============================================================

#[test]
fn reject_unsorted_dict_keys() {
    // d 4:beta i1e 5:alpha i2e e  — keys not sorted
    let data = b"d4:betai1e5:alphai2ee";
    let result = from_bytes::<BencodeValue>(data);
    assert!(result.is_err(), "should reject unsorted keys");
}

#[test]
fn reject_duplicate_dict_keys() {
    // d 1:a i1e 1:a i2e e — duplicate key
    let data = b"d1:ai1e1:ai2ee";
    let result = from_bytes::<BencodeValue>(data);
    assert!(result.is_err(), "should reject duplicate keys");
}

#[test]
fn reject_interior_minus() {
    // BEP 3: integers must be base-10 with optional leading minus.
    // A minus sign in the middle of a number is invalid.
    let data = b"i35412-5633e";
    assert!(
        from_bytes::<i64>(data).is_err(),
        "interior minus sign should be rejected"
    );
}

#[test]
fn reject_integer_overflow_21_digits() {
    // 184467440737095516154 is 21 digits and overflows i64
    // (i64::MAX = 9223372036854775807, 19 digits).
    let data = b"i184467440737095516154e";
    assert!(
        from_bytes::<i64>(data).is_err(),
        "21-digit integer overflowing i64 should be rejected"
    );
}

#[test]
fn info_hash_from_raw_bytes() {
    // Info hash MUST be computed from the original raw bytes of the info
    // dict, not from re-serialized bytes. Build a .torrent-like structure,
    // parse the info dict and re-serialize it, verifying the SHA-1 matches.
    let mut torrent = Vec::new();
    torrent.extend_from_slice(b"d");
    torrent.extend_from_slice(b"8:announce35:http://tracker.example.com/announce");
    torrent.extend_from_slice(b"4:info");

    let info_bytes_start = torrent.len();
    torrent.extend_from_slice(b"d");
    torrent.extend_from_slice(b"6:lengthi1048576e");
    torrent.extend_from_slice(b"4:name11:example.txt");
    torrent.extend_from_slice(b"12:piece lengthi262144e");
    torrent.extend_from_slice(b"6:pieces20:");
    torrent.extend_from_slice(&[0xAA; 20]); // fake piece hash
    torrent.extend_from_slice(b"e");
    let info_bytes_end = torrent.len();

    torrent.extend_from_slice(b"e");

    let raw_info = &torrent[info_bytes_start..info_bytes_end];

    // Parse the info dict and re-serialize it
    let parsed: BencodeValue = from_bytes(raw_info).unwrap();
    let reserialized = to_bytes(&parsed).unwrap();

    // For canonical bencode the re-serialized bytes must match the original
    assert_eq!(
        raw_info,
        &reserialized[..],
        "re-serialized info dict must match original raw bytes"
    );
}
