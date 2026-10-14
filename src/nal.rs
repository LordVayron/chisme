//! H.264 bitstream helpers: Annex B (start codes, what Media Foundation
//! encoders produce) to AVCC (length-prefixed, what FLV/RTMP and MP4 carry),
//! parameter sets and the `avcC` decoder configuration record.

/// NAL units of an Annex B buffer (without start codes).
pub fn split_annexb(data: &[u8]) -> Vec<&[u8]> {
    let mut starts = vec![];
    let mut i = 0;
    while i + 3 <= data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            starts.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut out = vec![];
    for (k, &s) in starts.iter().enumerate() {
        let mut end = starts.get(k + 1).map_or(data.len(), |&n| n - 3);
        // A 4-byte start code leaves a trailing zero on the previous unit.
        while end > s && k + 1 < starts.len() && data[end - 1] == 0 {
            end -= 1;
        }
        if end > s {
            out.push(&data[s..end]);
        }
    }
    out
}

fn nal_type(n: &[u8]) -> u8 {
    n.first().map_or(0, |b| b & 0x1f)
}

pub fn has_parameter_sets(data: &[u8]) -> bool {
    split_annexb(data).iter().any(|n| nal_type(n) == 7)
}

pub fn has_idr(data: &[u8]) -> bool {
    split_annexb(data).iter().any(|n| nal_type(n) == 5)
}

/// The first SPS and PPS in the buffer.
pub fn parameter_sets(data: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    let nals = split_annexb(data);
    let sps = nals.iter().find(|n| nal_type(n) == 7)?;
    let pps = nals.iter().find(|n| nal_type(n) == 8)?;
    Some((sps.to_vec(), pps.to_vec()))
}

/// The picture NAL units as AVCC (4-byte lengths). Parameter sets and
/// access unit delimiters are left out: they travel in the `avcC` record.
pub fn to_avcc(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 16);
    for n in split_annexb(data) {
        if matches!(nal_type(n), 7 | 8 | 9) {
            continue;
        }
        out.extend_from_slice(&(n.len() as u32).to_be_bytes());
        out.extend_from_slice(n);
    }
    out
}

/// AVCDecoderConfigurationRecord (the FLV sequence header / MP4 `avcC`).
pub fn avcc_record(sps: &[u8], pps: &[u8]) -> Vec<u8> {
    let mut r = vec![1, sps.get(1).copied().unwrap_or(100), sps.get(2).copied().unwrap_or(0), sps.get(3).copied().unwrap_or(40), 0xff, 0xe1];
    r.extend_from_slice(&(sps.len() as u16).to_be_bytes());
    r.extend_from_slice(sps);
    r.push(1);
    r.extend_from_slice(&(pps.len() as u16).to_be_bytes());
    r.extend_from_slice(pps);
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPS: [u8; 4] = [0x67, 0x64, 0x00, 0x28];
    const PPS: [u8; 2] = [0x68, 0xee];

    fn stream() -> Vec<u8> {
        let mut s = vec![0, 0, 0, 1, 0x09, 0xf0]; // AUD
        s.extend([0, 0, 0, 1]);
        s.extend(SPS);
        s.extend([0, 0, 1]);
        s.extend(PPS);
        s.extend([0, 0, 0, 1, 0x65, 0x88, 0x84, 0x00]); // IDR ending in a zero byte of data
        s.extend([0, 0, 1, 0x41, 0x9a]);
        s
    }

    #[test]
    fn splits_three_and_four_byte_start_codes() {
        let s = stream();
        let n = split_annexb(&s);
        assert_eq!(n, vec![&[0x09, 0xf0][..], &SPS, &PPS, &[0x65, 0x88, 0x84][..], &[0x41, 0x9a][..]]);
        assert!(has_parameter_sets(&s) && has_idr(&s));
        assert!(!has_idr(&[0, 0, 1, 0x41, 1]));
    }

    #[test]
    fn avcc_keeps_only_pictures() {
        let a = to_avcc(&stream());
        assert_eq!(a, vec![0, 0, 0, 3, 0x65, 0x88, 0x84, 0, 0, 0, 2, 0x41, 0x9a]);
        let (sps, pps) = parameter_sets(&stream()).unwrap();
        assert_eq!((sps.as_slice(), pps.as_slice()), (&SPS[..], &PPS[..]));
    }

    #[test]
    fn decoder_record() {
        let r = avcc_record(&SPS, &PPS);
        assert_eq!(&r[..6], &[1, 0x64, 0x00, 0x28, 0xff, 0xe1]);
        assert_eq!(&r[6..8], &[0, 4]);
        assert_eq!(&r[12..15], &[1, 0, 2]);
    }
}
