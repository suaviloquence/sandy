use self::data::{Layer, Version};
use super::{Codec, Song};
use crate::playlist::SongMetadata;
use futures::future::BoxFuture;
use futures::{AsyncWrite, AsyncWriteExt};
use id3::Id3;
use std::io::{self, BufRead, BufReader, Cursor, Read, Seek};

mod data;

#[derive(Debug, Clone, Copy)]
/// [0]: 0b11111111 - first part of sync word
/// [1]: 0b111vvllc where v is version, l is layer, c is error-protected
/// [2]: 0bBBBBsspP where B is bitrate idx, s is sample idx, p is padding existence, and P is for private use
pub struct Header([u8; 4]);

impl std::ops::Deref for Header {
    type Target = [u8; 4];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Header {
    #[inline]
    pub const fn sync(self) -> bool {
        self.0[0] == 0xFF && self.0[1] & 0b11100000 == 0b11100000
    }

    #[inline]
    pub const fn version(self) -> Version {
        match (self.0[1] >> 3) & 0b11 {
            0b11 => Version::V1,
            0b10 => Version::V2,
            0b00 => Version::V2_5,
            // invalid layer
            _ =>
            {
                #[allow(unconditional_panic)]
                [][1]
            }
        }
    }

    #[inline]
    pub const fn layer(self) -> Layer {
        match (self.0[1] >> 1) & 0b11 {
            0b11 => Layer::L1,
            0b10 => Layer::L2,
            0b01 => Layer::L3,
            // invalid layer, can't panic in const fn
            _ =>
            {
                #[allow(unconditional_panic)]
                [][1]
            }
        }
    }

    #[inline]
    pub const fn bitrate(self) -> i64 {
        data::get_bitrate(self.version(), self.layer(), self.0[2] >> 4) * 1000
    }

    #[inline]
    pub fn sample_rate(self) -> i64 {
        data::get_sample_rate(self.version(), (self[2] >> 2) & 0b11)
    }

    #[inline]
    pub fn padding(self) -> bool {
        self[2] & 0b10 == 0b10
    }

    #[inline]
    pub fn samples(self) -> i64 {
        data::get_samples_per_frame(self.version(), self.layer())
    }

    #[inline]
    pub fn frame_size(self) -> i64 {
        if self.layer() == Layer::L1 {
            4 * (12 * self.bitrate() / self.sample_rate() + self.padding() as i64)
        } else {
            144 * self.bitrate() / self.sample_rate() + self.padding() as i64
        }
    }

    #[inline]
    pub fn duration(self) -> f64 {
        (self.samples() as f64) / (self.sample_rate() as f64)
    }
}

#[derive(Debug, Clone)]
pub struct Frame {
    pub header: Header,
    pub data: Vec<u8>,
}

impl super::Frame for Frame {
    type Future = BoxFuture<'static, io::Result<usize>>;

    fn write<W: AsyncWrite + Unpin + Send>(&self, mut w: W) -> Self::Future {
        Box::pin(async move { Ok(w.write(&self.header[..]).await? + w.write(&self.data).await?) })
    }
    
    fn into_bytes(self) -> io::Result<Vec<u8>> {
        Ok(self.header.0.into_iter().chain(self.data.into_iter()).collect())
    }
}

#[derive(Debug)]
pub struct Mp3;

impl Codec for Mp3 {
    const MIME_TYPE: &'static str = "audio/mpeg";

    type Frame = Frame;
    type Iterator<'a> = FrameIterator<Cursor<&'a Vec<u8>>>;

    fn load(metadata: SongMetadata, data: impl Read + Seek) -> io::Result<Song<Self>> {
        let mut reader = BufReader::new(data);
        let mut data = Vec::new();
        reader.read_to_end(&mut data)?;
        reader.rewind()?;

        let _id3 = Id3::read(&mut reader)?;
        let duration = get_duration(&mut reader)?;
        let mut source = reader.into_inner();
        source.rewind()?;

        Ok(Song {
            metadata,
            data,
            duration,
            codec: Mp3,
        })
    }

    fn frames(song: &Song<Self>) -> Self::Iterator<'_> {
        let mut cursor = Cursor::new(&song.data);

        // skip ID3 tags
        let _ = Id3::read(&mut cursor);

        // move to next 0xFF
        for i in cursor.position() as usize..song.data.len() {
            if song.data[i] == 0xFF {
                cursor.set_position(i as u64);
                break;
            }
        }
        // if it gets to the end without finding seek word we have bigger problems

        FrameIterator { cursor }
    }
}

impl Song<Mp3> {
}
#[derive(Debug)]
struct FrameIterator<R> {
    cursor: R,
}

impl<R: Read> FrameIterator<R> {
    fn next_frame(&mut self) -> io::Result<Option<Frame>> {
        let mut header = [0u8; 4];

        if self.cursor.read(&mut header)? != header.len() {
            return Ok(None);
        }

        let header = Header(header);

        assert!(header.sync(), "Sync word not found");

        let mut data = vec![0u8; header.frame_size() as usize - header.len()];
        self.cursor.read_exact(&mut data)?;

        Ok(Some(Frame { header, data }))
    }
}

impl<R: Read> Iterator for FrameIterator<R> {
    type Item = Frame;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_frame().ok().flatten()
    }
}

/// Gets the duration of an MP3 (with ID3 tags already read, if they exist)
fn get_duration(mut source: impl BufRead + Seek) -> io::Result<f64> {
    let mut duration = 0.;
    let mut header = [0u8; 4];
    // TODO - ignore ID3 footer.

    let mut data = Vec::new();
    source.read_until(0xFF, &mut data)?;
    source.seek(io::SeekFrom::Current(-1))?;

    while source.read(&mut header)? == header.len() {
        let header = Header(header);
        assert!(header.sync(), "Sync word not found!");

        duration += header.duration();

        // skip header size
        source.seek(io::SeekFrom::Current(header.frame_size() - 4))?;
    }

    Ok(duration)
}
