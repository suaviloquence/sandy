use std::io::{self, Read, Seek, Write};

macro_rules! read_be {
    ($t: ty, $r: ident) => {{
        let mut buf = [0; (<$t>::BITS / 8) as usize];
        $r.read_exact(&mut buf).map(|_| <$t>::from_be_bytes(buf))
    }};
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[allow(non_camel_case_types)]
struct u28(u32);

macro_rules! last7 {
    ($x: expr) => {
        ($x & 0b01111111)
    };
}

impl u28 {
    /// reads (big-endian) from 4 bytes
    #[inline]
    const fn from(value: [u8; 4]) -> Self {
        let mut val = 0;

        val |= last7!(value[0]) as u32;
        val <<= 7;

        val |= last7!(value[1]) as u32;
        val <<= 7;

        val |= last7!(value[2]) as u32;
        val <<= 7;

        val |= last7!(value[3]) as u32;

        Self(val)
    }

    #[inline]
    const fn into(self) -> [u8; 4] {
        [
            last7!(self.0 >> 21) as u8,
            last7!(self.0 >> 14) as u8,
            last7!(self.0 >> 7) as u8,
            last7!(self.0) as u8,
        ]
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(u8)]
#[non_exhaustive]
pub enum HeaderFlags {
    Unsynchronization = 1 << 7,
    ExtendedHeader = 1 << 6,
    Experimental = 1 << 5,
    Footer = 1 << 4,
}

#[derive(Debug, Clone)]
// represents UTF-16 big-endian encoding
pub struct Text(Vec<u8>);

impl From<&str> for Text {
    fn from(value: &str) -> Self {
        Self(
            value
                .encode_utf16()
                .chain(std::iter::once(0))
                .flat_map(|x| x.to_be_bytes())
                .collect(),
        )
    }
}

impl From<Vec<u8>> for Text {
    fn from(mut value: Vec<u8>) -> Self {
        match (value[0], &value[1..=2]) {
            // (0): Latin-1; (3): UTF-8, terminated with \0
            (0 | 3, _) => Self::from(String::from_utf8_lossy(&value[1..value.len() - 1]).as_ref()),
            // UTF-16, w/ byte order mark, terminated with \0\0 ⇒ little endian
            (1, [0xFF, 0xFE]) => {
                let mut i = 1;
                // its ok if we swap the last 2 bytes
                while i < value.len() {
                    value[i] = value[i + 1];
                    i += 2;
                }

                Self(value)
            }
            // UTF-16 Big-Endian
            (1, [0xFE, 0xFF]) | (2, _) => Self(value),
            (enc, order) => unreachable!("Invalid text encoding: {} {:?}", enc, order),
        }
    }
}

impl Text {
    #[inline]
    pub fn byte_len(&self) -> u32 {
        // encoding bit + utf16 size (in bytes) + ending chars \0 \0 + final \0 to make it even
        1 + self.0.len() as u32
    }

    pub fn write(&self, mut w: impl Write) -> io::Result<usize> {
        let mut n = 0;
        // encoding:
        n += w.write(&[2u8])?;
        n += w.write(&self.0)?;
        Ok(n)
        // w.write_all(&[0u8])
    }
}

#[derive(Debug)]
pub enum FrameType {
    Title(Text),
    Artist(Text),
    Other { tag: [u8; 4], data: Vec<u8> },
}

#[derive(Debug)]
pub struct Frame {
    frame_type: FrameType,
    flags: [u8; 2],
}

impl FrameType {
    #[inline]
    pub const fn tag(&self) -> &[u8; 4] {
        match self {
            FrameType::Title(_) => b"TIT2",
            FrameType::Artist(_) => b"TPE1",
            FrameType::Other { tag, .. } => tag,
        }
    }

    #[inline]
    pub fn data_len(&self) -> u32 {
        match self {
            FrameType::Title(t) | FrameType::Artist(t) => t.byte_len(),
            FrameType::Other { data, .. } => data.len() as u32,
        }
    }

    #[inline]
    pub fn write_data(&self, mut w: impl Write) -> io::Result<usize> {
        match self {
            FrameType::Title(text) | FrameType::Artist(text) => text.write(w),
            FrameType::Other { data, .. } => w.write(data),
        }
    }
}

impl From<([u8; 4], Vec<u8>)> for FrameType {
    fn from((tag, data): ([u8; 4], Vec<u8>)) -> Self {
        match &tag {
            b"TIT2" => Self::Title(Text::from(data)),
            _ => Self::Other { tag, data },
        }
    }
}

impl Frame {
    pub fn byte_len(&self) -> u32 {
        // header is 10 bytes - frame id (4) + flags (2) + size (u32 ⇒ 4)
        self.frame_type.data_len() + 10
    }

    pub fn read(mut r: impl Read) -> io::Result<Option<Self>> {
        let mut frame_id = [0u8; 4];
        match r.read(&mut frame_id) {
            Ok(4) => (),
            Ok(_) => return Ok(None),
            Err(e) => return Err(e),
        };

        let size = read_be!(u32, r)?;

        let mut flags = [0u8; 2];
        r.read_exact(&mut flags)?;

        let mut data = vec![0; size as usize];
        r.read_exact(&mut data)?;

        Ok(Some(Self {
            flags,
            frame_type: FrameType::from((frame_id, data)),
        }))
    }

    pub fn write(&self, mut w: impl Write) -> io::Result<usize> {
        let mut n = 0;
        n += w.write(&self.frame_type.tag()[..])?;

        n += w.write(&self.frame_type.data_len().to_be_bytes())?;
        n += w.write(&self.flags)?;
        n += self.frame_type.write_data(w)?;

        Ok(n)
    }
}

#[derive(Debug)]
pub struct Id3 {
    major_version: u8,
    revision: u8,
    flags: u8,
    frames: Vec<Frame>,
}

impl Id3 {
    pub fn read(mut source: impl Read + Seek) -> io::Result<Option<Self>> {
        let mut header_size = 10;

        let mut version = [0u8; 5];
        source.read_exact(&mut version)?;

        // Check if header found
        if &version[0..3] != b"ID3" {
            source.rewind()?;
            return Ok(None);
        }

        // minor (revision) version is version[4]
        assert_eq!(&version[0..3], b"ID3", "Invalid ID3 tag/version");
        assert!((3..=4).contains(&version[3]));

        let flags = read_be!(u8, source)?;

        // size is an absoultely wild 28-bit integer where the leading bit of each octet is ignored
        let mut size = [0u8; 4];
        source.read_exact(&mut size)?;
        let size = u28::from(size).0;

        if flags & (HeaderFlags::ExtendedHeader as u8) != 0 {
            let ext_header_size = read_be!(u32, source)?;
            // TODO: handle extended header; now we just skip it
            source
                // first 4 bytes are already-read ext_header_size
                .seek(io::SeekFrom::Current(ext_header_size as i64 - 4))?;

            header_size += ext_header_size;
        }

        let mut src = (&mut source).take((size - header_size) as u64);

        let mut frames = Vec::new();

        while let Some(frame) = Frame::read(&mut src)? {
            frames.push(frame)
        }

        Ok(Some(Self {
            major_version: version[3],
            revision: version[4],
            flags,
            frames,
        }))
    }

    pub fn as_bytes(&self) -> Vec<u8> {
        let mut vec = vec![
            b'I',
            b'D',
            b'3',
            self.major_version,
            self.revision,
            self.flags,
        ];

        let size: u32 = self.frames.iter().map(Frame::byte_len).sum();
        vec.extend(u28(size).into());

        for frame in &self.frames {
            frame.write(&mut vec).expect("Error writing to vec");
        }

        vec
    }
}
