use std::net::IpAddr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SybilPool {
    Bep42,
    Random,
}

const POLY: u32 = 0x82F6_3B78;

pub fn crc32c(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ POLY
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn masked_ip(ip: IpAddr) -> Vec<u8> {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            vec![o[0] & 0x03, o[1] & 0x0f, o[2] & 0x3f, o[3]]
        }
        IpAddr::V6(v6) => {
            let o = v6.octets();
            let mask = [0x01, 0x03, 0x07, 0x0f, 0x1f, 0x3f, 0x7f, 0xff];
            (0..8).map(|i| o[i] & mask[i]).collect()
        }
    }
}

fn bep42_crc(ip: IpAddr, rand: u8) -> u32 {
    let mut bytes = masked_ip(ip);
    let r = rand & 0x07;
    bytes[0] |= r << 5;
    crc32c(&bytes)
}

#[allow(dead_code)]
pub fn bep42_prefix(ip: IpAddr, rand: u8) -> [u8; 3] {
    let crc = bep42_crc(ip, rand);
    [
        (crc >> 24) as u8,
        (crc >> 16) as u8,
        ((crc >> 8) as u8) & 0xf8,
    ]
}

pub fn bep42_node_id(ip: IpAddr, mut next: impl FnMut() -> u8) -> [u8; 20] {
    let rand = next();
    let crc = bep42_crc(ip, rand);
    let mut id = [0u8; 20];
    id[0] = (crc >> 24) as u8;
    id[1] = (crc >> 16) as u8;
    id[2] = (((crc >> 8) as u8) & 0xf8) | (next() & 0x07);
    for b in id.iter_mut().skip(3).take(16) {
        *b = next();
    }
    id[19] = rand;
    id
}

pub fn bep42_node_id_rng(ip: IpAddr) -> [u8; 20] {
    bep42_node_id(ip, rand::random::<u8>)
}

pub fn random_node_id() -> [u8; 20] {
    rand::random::<[u8; 20]>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn bep42_test_vector_1() {
        let ip = IpAddr::V4(Ipv4Addr::new(124, 31, 75, 21));
        let p = bep42_prefix(ip, 1);
        assert_eq!(p[0], 0x5f);
        assert_eq!(p[1], 0xbf);
        assert_eq!(p[2], 0xb8);
    }

    #[test]
    fn bep42_test_vector_2() {
        let ip = IpAddr::V4(Ipv4Addr::new(21, 75, 31, 124));
        let p = bep42_prefix(ip, 86);
        assert_eq!(p[0], 0x5a);
        assert_eq!(p[1], 0x3c);
        assert_eq!(p[2], 0xe8);
    }

    #[test]
    fn bep42_test_vector_3() {
        let ip = IpAddr::V4(Ipv4Addr::new(65, 23, 51, 170));
        let p = bep42_prefix(ip, 22);
        assert_eq!(p[0], 0xa5);
        assert_eq!(p[1], 0xd4);
        assert_eq!(p[2], 0x30);
    }

    #[test]
    fn bep42_test_vector_4() {
        let ip = IpAddr::V4(Ipv4Addr::new(84, 124, 73, 14));
        let p = bep42_prefix(ip, 65);
        assert_eq!(p[0], 0x1b);
        assert_eq!(p[1], 0x03);
        assert_eq!(p[2], 0x20);
    }

    #[test]
    fn bep42_test_vector_5() {
        let ip = IpAddr::V4(Ipv4Addr::new(43, 213, 53, 83));
        let p = bep42_prefix(ip, 90);
        assert_eq!(p[0], 0xe5);
        assert_eq!(p[1], 0x6f);
        assert_eq!(p[2], 0x68);
    }

    #[test]
    fn crc32c_known() {
        assert_eq!(crc32c(b"123456789"), 0xE3069283);
    }
}
