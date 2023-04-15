//! MP3 header data lookup tables
//!
//! see: https://www.datavoyage.com/mpgscript/mpeghdr.htm

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Version {
    V1,
    V2,
    V2_5,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    L1,
    L2,
    L3,
}
use Layer::*;
use Version::*;

/// Returns the bitrate (kb/s) for the index where the first four bits represent the MPEG version and layer, and the last four bits represent the bitrate index for that version/layer combo.
/// Versions: 00 => 2.5 (unimplemented), 01 => invalid, 10 => V2, 11 => V1
/// Layers: 00 => invalid, 01 => L3, 10 => L2, 11 => L1
#[inline]
pub(super) const fn get_bitrate(ver: Version, layer: Layer, bitrate_idx: u8) -> i64 {
    let bitrate_idx = ((bitrate_idx - 1) & 0b1111) as usize;
    (match (ver, layer) {
        (V1, L1) => [
            32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448,
        ],
        (V1, L2) => [
            32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384,
        ],
        (V1, L3) => [
            32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
        ],
        (V2 | V2_5, L1) => [
            32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256,
        ],
        (V2 | V2_5, L2 | L3) => [8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160],
    })[bitrate_idx]
}

/// Gets the sample rate, in Hz, of the frame depending on the MPEG version and the sample index
pub(super) const fn get_sample_rate(ver: Version, sample_idx: u8) -> i64 {
    let sample_idx = (sample_idx & 0b11) as usize;
    (match ver {
        V1 => [44100, 48000, 32000],
        V2 => [22050, 24000, 16000],
        V2_5 => [11025, 12000, 8000],
    })[sample_idx]
}

pub(super) const fn get_samples_per_frame(version: Version, layer: Layer) -> i64 {
    match (version, layer) {
        (V1, L1) => 384,
        (V1, L2 | L3) => 1152,
        (V2 | V2_5, L1) => 384,
        (V2 | V2_5, L2) => 1152,
        (V2 | V2_5, L3) => 476,
    }
}
