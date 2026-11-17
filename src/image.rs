//! Images in user messages (RFC 0034): the type from the first bytes, the
//! size from the header, the limits, and the hash they are saved under. No
//! image crate: nothing is decoded, resized or converted.

use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// The largest image, in bytes.
pub const MAX_BYTES: usize = 5 << 20;
/// Images in one message.
pub const MAX_PER_MESSAGE: usize = 10;
/// Bytes of all images of one message: in base64 a third more, and the
/// `send` must stay below the protocol's 16 MiB per message.
pub const MAX_MESSAGE_BYTES: usize = 11 << 20;

/// The image types models take.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Kind {
    #[serde(rename = "image/png")]
    Png,
    #[serde(rename = "image/jpeg")]
    Jpeg,
    #[serde(rename = "image/gif")]
    Gif,
    #[serde(rename = "image/webp")]
    Webp,
}

impl Kind {
    pub fn media_type(self) -> &'static str {
        match self {
            Kind::Png => "image/png",
            Kind::Jpeg => "image/jpeg",
            Kind::Gif => "image/gif",
            Kind::Webp => "image/webp",
        }
    }

    pub fn from_media_type(media_type: &str) -> Option<Kind> {
        [Kind::Png, Kind::Jpeg, Kind::Gif, Kind::Webp]
            .into_iter()
            .find(|k| k.media_type() == media_type)
    }

    pub fn extension(self) -> &'static str {
        match self {
            Kind::Png => "png",
            Kind::Jpeg => "jpg",
            Kind::Gif => "gif",
            Kind::Webp => "webp",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Png => "PNG",
            Kind::Jpeg => "JPEG",
            Kind::Gif => "GIF",
            Kind::Webp => "WebP",
        }
    }

    /// The type of `bytes`, from their first bytes.
    pub fn sniff(bytes: &[u8]) -> Option<Kind> {
        if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            Some(Kind::Png)
        } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
            Some(Kind::Jpeg)
        } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
            Some(Kind::Gif)
        } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
            Some(Kind::Webp)
        } else {
            None
        }
    }
}

/// An image. Saved and sent to clients without its bytes: the session keeps
/// them in `images/<hash>.<ext>` (see `spec/sessions.md`, "Persistence").
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Image {
    pub media_type: Kind,
    pub width: u32,
    pub height: u32,
    /// Bytes.
    pub size: u64,
    /// SHA-256 of the bytes, in hex.
    pub hash: String,
    /// Empty when not loaded (a file that went missing).
    #[serde(skip, default = "no_data")]
    pub data: Arc<[u8]>,
}

fn no_data() -> Arc<[u8]> {
    Arc::from(Vec::new())
}

impl fmt::Debug for Image {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Image")
            .field("media_type", &self.media_type)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("size", &self.size)
            .field("hash", &self.hash)
            .field("loaded", &self.loaded())
            .finish()
    }
}

impl Image {
    /// Checks `bytes` and makes an image of them; the error is for the
    /// user.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Image, String> {
        let kind = Kind::sniff(&bytes).ok_or("not an image (PNG, JPEG, GIF or WebP)")?;
        if bytes.len() > MAX_BYTES {
            return Err(format!(
                "the image is {}, more than {}; crop it or make it smaller",
                size_text(bytes.len() as u64),
                size_text(MAX_BYTES as u64)
            ));
        }
        let (width, height) = dimensions(kind, &bytes)
            .filter(|&(w, h)| w > 0 && h > 0)
            .ok_or_else(|| format!("cannot read the size of this {} image", kind.name()))?;
        Ok(Image {
            media_type: kind,
            width,
            height,
            size: bytes.len() as u64,
            hash: hex(&sha256(&bytes)),
            data: Arc::from(bytes),
        })
    }

    /// Whether the bytes are there.
    pub fn loaded(&self) -> bool {
        !self.data.is_empty() && self.data.len() as u64 == self.size
    }

    /// The bytes in base64, as the APIs want them.
    pub fn base64(&self) -> String {
        base64(&self.data)
    }

    /// `PNG 1366×306 48 KB`.
    pub fn describe(&self) -> String {
        format!(
            "{} {}×{} {}",
            self.media_type.name(),
            self.width,
            self.height,
            size_text(self.size)
        )
    }

    /// The file it is saved in.
    pub fn file_name(&self) -> String {
        format!("{}.{}", self.hash, self.media_type.extension())
    }

    /// Fills in the bytes read from its file; `false` if they do not
    /// match.
    pub fn load(&mut self, bytes: Vec<u8>) -> bool {
        if bytes.len() as u64 != self.size || hex(&sha256(&bytes)) != self.hash {
            return false;
        }
        self.data = Arc::from(bytes);
        true
    }
}

/// An image of a message with the number of its placeholder `[Image #N]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Numbered {
    pub number: u32,
    #[serde(flatten)]
    pub image: Image,
}

/// The placeholder of image `number` in the text.
pub fn placeholder(number: u32) -> String {
    format!("[Image #{number}]")
}

/// `48 KB`, `1.2 MB`.
pub fn size_text(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{} KB", bytes.div_ceil(1024))
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// Checks the images of one message against the limits.
pub fn check_message(images: &[Numbered]) -> Result<(), String> {
    if images.len() > MAX_PER_MESSAGE {
        return Err(format!(
            "{} images; a message takes at most {MAX_PER_MESSAGE}",
            images.len()
        ));
    }
    let total: u64 = images.iter().map(|i| i.image.size).sum();
    if total > MAX_MESSAGE_BYTES as u64 {
        return Err(format!(
            "the images are {} together; a message takes at most {}",
            size_text(total),
            size_text(MAX_MESSAGE_BYTES as u64)
        ));
    }
    Ok(())
}

// The size from the header.

fn be16(bytes: &[u8], at: usize) -> Option<u32> {
    let b = bytes.get(at..at + 2)?;
    Some(u32::from(u16::from_be_bytes([*b.first()?, *b.get(1)?])))
}

fn le16(bytes: &[u8], at: usize) -> Option<u32> {
    let b = bytes.get(at..at + 2)?;
    Some(u32::from(u16::from_le_bytes([*b.first()?, *b.get(1)?])))
}

fn be32(bytes: &[u8], at: usize) -> Option<u32> {
    Some((be16(bytes, at)? << 16) | be16(bytes, at + 2)?)
}

fn le24(bytes: &[u8], at: usize) -> Option<u32> {
    let b = bytes.get(at..at + 3)?;
    Some(u32::from(*b.first()?) | (u32::from(*b.get(1)?) << 8) | (u32::from(*b.get(2)?) << 16))
}

/// Width and height from the header.
pub fn dimensions(kind: Kind, bytes: &[u8]) -> Option<(u32, u32)> {
    match kind {
        Kind::Png => {
            // The first chunk is IHDR: width and height after its type.
            if bytes.get(12..16)? != b"IHDR" {
                return None;
            }
            Some((be32(bytes, 16)?, be32(bytes, 20)?))
        }
        Kind::Gif => Some((le16(bytes, 6)?, le16(bytes, 8)?)),
        Kind::Jpeg => jpeg_dimensions(bytes),
        Kind::Webp => webp_dimensions(bytes),
    }
}

fn jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut at = 2;
    loop {
        // Markers may be padded with 0xff.
        while *bytes.get(at)? == 0xff && *bytes.get(at + 1)? == 0xff {
            at += 1;
        }
        if *bytes.get(at)? != 0xff {
            return None;
        }
        let marker = *bytes.get(at + 1)?;
        at += 2;
        // Markers without a length.
        if marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
            continue;
        }
        let length = usize::try_from(be16(bytes, at)?).ok()?;
        // Start of frame, except DHT, JPG and DAC.
        if (0xc0..=0xcf).contains(&marker) && ![0xc4, 0xc8, 0xcc].contains(&marker) {
            return Some((be16(bytes, at + 5)?, be16(bytes, at + 3)?));
        }
        if marker == 0xd9 || marker == 0xda || length < 2 {
            return None;
        }
        at += length;
    }
}

fn webp_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    match bytes.get(12..16)? {
        b"VP8 " => {
            // After the frame tag and the start code: 14 bits each.
            if bytes.get(23..26)? != [0x9d, 0x01, 0x2a] {
                return None;
            }
            Some((le16(bytes, 26)? & 0x3fff, le16(bytes, 28)? & 0x3fff))
        }
        b"VP8L" => {
            if *bytes.get(20)? != 0x2f {
                return None;
            }
            let bits = u32::from_le_bytes(bytes.get(21..25)?.try_into().ok()?);
            Some(((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1))
        }
        b"VP8X" => Some((le24(bytes, 24)? + 1, le24(bytes, 27)? + 1)),
        _ => None,
    }
}

// Base64 and SHA-256: small enough to keep here instead of two crates.

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    let symbol = |n: u32| char::from(BASE64.get((n & 63) as usize).copied().unwrap_or(b'A'));
    for chunk in bytes.chunks(3) {
        let b = |i: usize| u32::from(chunk.get(i).copied().unwrap_or(0));
        let n = (b(0) << 16) | (b(1) << 8) | b(2);
        out.push(symbol(n >> 18));
        out.push(symbol(n >> 12));
        out.push(if chunk.len() > 1 { symbol(n >> 6) } else { '=' });
        out.push(if chunk.len() > 2 { symbol(n) } else { '=' });
    }
    out
}

/// Standard base64 with padding; `None` for anything else.
pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let value = |c: u8| -> Option<u32> {
        BASE64
            .iter()
            .position(|&s| s == c)
            .and_then(|p| u32::try_from(p).ok())
    };
    let bytes = text.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    let chunks = bytes.len() / 4;
    for (i, chunk) in bytes.chunks(4).enumerate() {
        let last = i + 1 == chunks;
        let pad = chunk.iter().rev().take_while(|&&c| c == b'=').count();
        if pad > 2 || (pad > 0 && !last) {
            return None;
        }
        let mut n = 0u32;
        for (j, &c) in chunk.iter().enumerate() {
            let v = if j >= 4 - pad { 0 } else { value(c)? };
            n = (n << 6) | v;
        }
        let [_, a, b, c] = n.to_be_bytes();
        out.push(a);
        if pad < 2 {
            out.push(b);
        }
        if pad < 1 {
            out.push(c);
        }
    }
    Some(out)
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut message = bytes.to_vec();
    let bits = (bytes.len() as u64).wrapping_mul(8);
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bits.to_be_bytes());
    for block in message.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for (slot, word) in w.iter_mut().zip(block.as_chunks::<4>().0) {
            *slot = u32::from_be_bytes(*word);
        }
        for i in 16..64 {
            let at = |j: usize| w.get(j).copied().unwrap_or(0);
            let (w15, w2) = (at(i - 15), at(i - 2));
            let s0 = w15.rotate_right(7) ^ w15.rotate_right(18) ^ (w15 >> 3);
            let s1 = w2.rotate_right(17) ^ w2.rotate_right(19) ^ (w2 >> 10);
            let value = at(i - 16)
                .wrapping_add(s0)
                .wrapping_add(at(i - 7))
                .wrapping_add(s1);
            if let Some(slot) = w.get_mut(i) {
                *slot = value;
            }
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for (k, wi) in K.iter().zip(w.iter()) {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(*k)
                .wrapping_add(*wi);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, v) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *slot = slot.wrapping_add(v);
        }
    }
    let mut out = [0u8; 32];
    for (chunk, word) in out.as_chunks_mut::<4>().0.iter_mut().zip(h) {
        *chunk = word.to_be_bytes();
    }
    out
}

/// Test images: the smallest headers each format needs.
#[cfg(test)]
pub mod fixtures {
    /// A PNG of `width` × `height`; only the header is real.
    pub fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
        bytes
    }

    pub fn gif(width: u16, height: u16) -> Vec<u8> {
        let mut bytes = b"GIF89a".to_vec();
        bytes.extend_from_slice(&width.to_le_bytes());
        bytes.extend_from_slice(&height.to_le_bytes());
        bytes.extend_from_slice(&[0, 0, 0]);
        bytes
    }

    pub fn jpeg(width: u16, height: u16) -> Vec<u8> {
        let mut bytes = vec![0xff, 0xd8];
        // An APP0 segment first, then the frame.
        bytes.extend_from_slice(&[0xff, 0xe0, 0x00, 0x04, 0x4a, 0x46]);
        bytes.extend_from_slice(&[0xff, 0xc0, 0x00, 0x11, 0x08]);
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&[3; 10]);
        bytes
    }

    pub fn webp_extended(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = b"RIFF\0\0\0\0WEBPVP8X\x0a\0\0\0\0\0\0\0".to_vec();
        bytes.extend_from_slice(&(width - 1).to_le_bytes()[..3]);
        bytes.extend_from_slice(&(height - 1).to_le_bytes()[..3]);
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;

    #[test]
    fn sha256_known_values() {
        assert_eq!(
            hex(&sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        // Two blocks.
        assert_eq!(
            hex(&sha256(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn base64_both_ways() {
        for (plain, coded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
        ] {
            assert_eq!(base64(plain.as_bytes()), coded);
            assert_eq!(base64_decode(coded).unwrap(), plain.as_bytes());
        }
        let bytes: Vec<u8> = (0..=255).collect();
        assert_eq!(base64_decode(&base64(&bytes)).unwrap(), bytes);
        for bad in ["Zg=", "Z===", "Zg==Zg==", "Z!==", "Zm9v\n"] {
            assert_eq!(base64_decode(bad), None, "{bad}");
        }
    }

    #[test]
    fn types_and_sizes() {
        let png = Image::from_bytes(png(1366, 306)).unwrap();
        assert_eq!(
            (png.media_type, png.width, png.height),
            (Kind::Png, 1366, 306)
        );
        assert_eq!(png.describe(), "PNG 1366×306 33 B");
        let gif = Image::from_bytes(gif(20, 10)).unwrap();
        assert_eq!((gif.media_type, gif.width, gif.height), (Kind::Gif, 20, 10));
        let jpeg = Image::from_bytes(jpeg(800, 600)).unwrap();
        assert_eq!(
            (jpeg.media_type, jpeg.width, jpeg.height),
            (Kind::Jpeg, 800, 600)
        );
        let webp = Image::from_bytes(webp_extended(640, 480)).unwrap();
        assert_eq!(
            (webp.media_type, webp.width, webp.height),
            (Kind::Webp, 640, 480)
        );
        assert_eq!(webp.file_name(), format!("{}.webp", webp.hash));
    }

    #[test]
    fn webp_lossy_and_lossless() {
        let mut lossy = b"RIFF\0\0\0\0WEBPVP8 \0\0\0\0\0\0\0\x9d\x01\x2a".to_vec();
        lossy.extend_from_slice(&300u16.to_le_bytes());
        lossy.extend_from_slice(&200u16.to_le_bytes());
        assert_eq!(dimensions(Kind::Webp, &lossy), Some((300, 200)));
        let mut lossless = b"RIFF\0\0\0\0WEBPVP8L\0\0\0\0\x2f".to_vec();
        let bits: u32 = (300 - 1) | ((200 - 1) << 14);
        lossless.extend_from_slice(&bits.to_le_bytes());
        assert_eq!(dimensions(Kind::Webp, &lossless), Some((300, 200)));
    }

    #[test]
    fn refusals() {
        assert_eq!(
            Image::from_bytes(b"hello".to_vec()).unwrap_err(),
            "not an image (PNG, JPEG, GIF or WebP)"
        );
        // A header cut short.
        let e = Image::from_bytes(png(1, 1)[..18].to_vec()).unwrap_err();
        assert_eq!(e, "cannot read the size of this PNG image");
        assert!(Image::from_bytes(jpeg(0, 10)).is_err());
        let mut big = png(10, 10);
        big.resize(MAX_BYTES + 1, 0);
        let e = Image::from_bytes(big).unwrap_err();
        assert!(
            e.starts_with("the image is 5.0 MB, more than 5.0 MB"),
            "{e}"
        );
    }

    #[test]
    fn saved_without_bytes_and_loaded_again() {
        let image = Image::from_bytes(png(2, 3)).unwrap();
        let json = serde_json::to_value(&image).unwrap();
        assert_eq!(json["media_type"], "image/png");
        assert!(json.get("data").is_none());
        let mut back: Image = serde_json::from_value(json).unwrap();
        assert!(!back.loaded());
        assert!(!back.load(b"other".to_vec()));
        assert!(back.load(png(2, 3)));
        assert!(back.loaded());
        assert_eq!(back, image);
    }

    #[test]
    fn message_limits() {
        let one = Numbered {
            number: 1,
            image: Image::from_bytes(png(1, 1)).unwrap(),
        };
        assert!(check_message(&vec![one.clone(); 10]).is_ok());
        assert!(check_message(&vec![one.clone(); 11]).is_err());
        let mut big = one;
        big.image.size = 6 << 20;
        let e = check_message(&[big.clone(), big]).unwrap_err();
        assert!(e.contains("12.0 MB together"), "{e}");
        // The largest message fits in the protocol's limit.
        let base64 = MAX_MESSAGE_BYTES.div_ceil(3) * 4;
        assert!(base64 + 64 * 1024 < crate::protocol::MAX_LINE);
    }

    #[test]
    fn sizes_as_text() {
        assert_eq!(size_text(512), "512 B");
        assert_eq!(size_text(48 * 1024), "48 KB");
        assert_eq!(size_text(1_300_000), "1.2 MB");
    }
}
