//! Bounded MP4 container inspection. This does not decode compressed frames.
use crate::{AppError, AppResult};
use mp4parse::{CodecType, SampleEntry, TrackType};
use serde::{Deserialize, Serialize};
use std::io::Cursor;

pub const MAX_VIDEO_BYTES: usize = 16 * 1024 * 1024;
const MAX_SAMPLES: u32 = 65536;
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct VideoInfo {
    pub width: u16,
    pub height: u16,
    pub duration_ms: u64,
    pub audio: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VideoAsset {
    pub id: String,
    pub account_id: String,
    pub sha256: String,
    pub size: usize,
    pub video: VideoInfo,
    pub created_at: String,
    pub expires_at: String,
}
pub struct ProtectedVideo {
    pub asset: VideoAsset,
    pub wrapped_key: Vec<u8>,
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
}
pub struct VideoBinding<'a> {
    pub observation_id: &'a str,
    pub connection_revision: i64,
    pub key_revision: i64,
    pub proof: &'a str,
}
pub fn inspect_send_mp4(name: &str, bytes: &[u8]) -> AppResult<VideoInfo> {
    crate::file::validate_file_name(name)?;
    let info = inspect_mp4_codecs(bytes, false)?;
    if !name.to_ascii_lowercase().ends_with(".mp4")
        || bytes.len() > crate::file::MAX_FILE_BYTES
        || !(1000..=60000).contains(&info.duration_ms)
    {
        return Err(AppError::new(
            "UNSUPPORTED_VIDEO",
            "发送视频须为 .mp4 H.264 / AAC，1–60秒且最多8MiB",
        ));
    }
    Ok(info)
}
fn invalid() -> AppError {
    AppError::new(
        "UNSUPPORTED_VIDEO",
        "视频须为完整的 H.264 或 HEVC / AAC MP4，最多16MiB、10分钟及4096边长",
    )
}
fn u32_at(bytes: &[u8], offset: usize) -> AppResult<u32> {
    Ok(u32::from_be_bytes(
        bytes
            .get(offset..offset + 4)
            .ok_or_else(invalid)?
            .try_into()
            .map_err(|_| invalid())?,
    ))
}
struct BoxBudget {
    boxes: usize,
    moov: usize,
    tracks: usize,
    mdat: Vec<std::ops::Range<u64>>,
    sample_counts: Vec<u32>,
    ignored_metadata: Vec<usize>,
}
// mp4parse allocates some tables from their declared count before reading them.
// Validate those counts against the containing box and a shared budget first.
fn preflight(bytes: &[u8], base: usize, depth: usize, budget: &mut BoxBudget) -> AppResult<()> {
    if depth > 8 {
        return Err(invalid());
    }
    let mut offset = 0;
    while offset < bytes.len() {
        budget.boxes += 1;
        if budget.boxes > 4096 {
            return Err(invalid());
        }
        let size32 = u32_at(bytes, offset)?;
        let name = bytes.get(offset + 4..offset + 8).ok_or_else(invalid)?;
        let header = if size32 == 1 { 16 } else { 8 };
        let size = match size32 {
            0 if depth == 0 && name == b"mdat" => bytes.len() - offset,
            0 => return Err(invalid()),
            1 => usize::try_from(u64::from_be_bytes(
                bytes
                    .get(offset + 8..offset + 16)
                    .ok_or_else(invalid)?
                    .try_into()
                    .map_err(|_| invalid())?,
            ))
            .map_err(|_| invalid())?,
            n => n as usize,
        };
        if size < header || size > bytes.len() - offset {
            return Err(invalid());
        }
        let body = &bytes[offset + header..offset + size];
        if matches!(name, b"udta" | b"meta") {
            budget.ignored_metadata.push(base + offset + 4);
        }
        if depth == 0 && name == b"mdat" {
            budget
                .mdat
                .push((base + offset + header) as u64..(base + offset + size) as u64);
        }
        if name == b"moov" {
            if depth != 0 {
                return Err(invalid());
            }
            budget.moov += 1;
        }
        if name == b"trak" {
            budget.tracks += 1;
            if budget.tracks > 2 {
                return Err(invalid());
            }
        }
        if matches!(name, b"mvex" | b"moof" | b"pssh") {
            return Err(invalid());
        }
        let table = match name {
            b"stts" | b"ctts" => Some((4, 8, 8)),
            b"stsc" => Some((4, 12, 8)),
            b"stco" | b"stss" => Some((4, 4, 8)),
            b"co64" => Some((4, 8, 8)),
            b"stsz" => Some((8, if u32_at(body, 4)? == 0 { 4 } else { 0 }, 12)),
            b"stsd" | b"dref" => Some((4, 8, 8)),
            _ => None,
        };
        if let Some((at, stride, start)) = table {
            let count = u32_at(body, at)?;
            if name == b"stsz" {
                budget.sample_counts.push(count);
            }
            if count > MAX_SAMPLES
                || (count as usize)
                    .checked_mul(stride)
                    .and_then(|n| n.checked_add(start))
                    .is_none_or(|n| n > body.len())
            {
                return Err(invalid());
            }
            if matches!(name, b"stsd" | b"dref") && count != 1 {
                return Err(invalid());
            }
            if name == b"stsd" && body.get(22..24) != Some(&[0, 1]) {
                return Err(invalid());
            }
            if name == b"dref"
                && (body.len() != 20 || &body[12..16] != b"url " || u32_at(body, 16)? != 1)
            {
                return Err(invalid());
            }
        }
        if matches!(
            name,
            b"moov" | b"trak" | b"mdia" | b"minf" | b"stbl" | b"edts" | b"dinf"
        ) {
            preflight(body, base + offset + header, depth + 1, budget)?;
        }
        offset += size;
    }
    Ok(())
}
pub fn inspect_mp4(bytes: &[u8]) -> AppResult<VideoInfo> {
    inspect_mp4_codecs(bytes, true)
}
fn inspect_mp4_codecs(bytes: &[u8], allow_hevc: bool) -> AppResult<VideoInfo> {
    if bytes.len() < 32 || bytes.len() > MAX_VIDEO_BYTES || bytes.get(4..8) != Some(b"ftyp") {
        return Err(invalid());
    }
    let mut budget = BoxBudget {
        boxes: 0,
        moov: 0,
        tracks: 0,
        mdat: Vec::new(),
        sample_counts: Vec::new(),
        ignored_metadata: Vec::new(),
    };
    preflight(bytes, 0, 0, &mut budget)?;
    if budget.moov != 1 || budget.mdat.is_empty() {
        return Err(invalid());
    }
    // Skip optional user metadata in the parser input without shifting offsets.
    // Original bytes are retained for playback and their authenticated digest.
    let mut parser_input = zeroize::Zeroizing::new(bytes.to_vec());
    for offset in &budget.ignored_metadata {
        parser_input[*offset..*offset + 4].copy_from_slice(b"free");
    }
    let parsed =
        mp4parse::read_mp4(&mut Cursor::new(parser_input.as_slice())).map_err(|_| invalid())?;
    if parsed.mvex.is_some()
        || !parsed.psshs.is_empty()
        || parsed.tracks.is_empty()
        || parsed.tracks.len() > 2
    {
        return Err(invalid());
    }
    let (mut video, mut audio) = (None, false);
    if parsed.tracks.len() != budget.sample_counts.len() {
        return Err(invalid());
    }
    for (track_index, track) in parsed.tracks.iter().enumerate() {
        let scale = track.timescale.ok_or_else(invalid)?.0;
        let duration = track.duration.ok_or_else(invalid)?.0;
        let duration_ms = duration
            .checked_mul(1000)
            .and_then(|n| n.checked_div(scale))
            .ok_or_else(invalid)?;
        if !(1..=600000).contains(&duration_ms) {
            return Err(invalid());
        }
        let entries = &track.stsd.as_ref().ok_or_else(invalid)?.descriptions;
        if entries.len() != 1 {
            return Err(invalid());
        }
        match (&track.track_type, &entries[0]) {
            (TrackType::Video, SampleEntry::Video(v))
                if video.is_none()
                    && (v.codec_type == CodecType::H264
                        || (allow_hevc && v.codec_type == CodecType::HEVC))
                    && v.protection_info.is_empty() =>
            {
                if v.width == 0
                    || v.height == 0
                    || v.width > 4096
                    || v.height > 4096
                    || u32::from(v.width) * u32::from(v.height) > 8388608
                {
                    return Err(invalid());
                }
                video = Some(VideoInfo {
                    width: v.width,
                    height: v.height,
                    duration_ms,
                    audio: false,
                });
            }
            (TrackType::Audio, SampleEntry::Audio(a))
                if !audio
                    && a.codec_type == CodecType::AAC
                    && a.protection_info.is_empty()
                    && (1..=2).contains(&a.channelcount)
                    && a.samplerate > 0.0
                    && a.samplerate <= 96000.0 =>
            {
                audio = true
            }
            _ => return Err(invalid()),
        }
        let sizes = track.stsz.as_ref().ok_or_else(invalid)?;
        let chunks = track.stco.as_ref().ok_or_else(invalid)?;
        let runs = &track.stsc.as_ref().ok_or_else(invalid)?.samples;
        let samples = track
            .stts
            .as_ref()
            .ok_or_else(invalid)?
            .samples
            .iter()
            .try_fold(0u32, |sum, s| {
                if s.sample_delta > 0 {
                    sum.checked_add(s.sample_count)
                } else {
                    None
                }
            })
            .ok_or_else(invalid)?;
        if samples == 0
            || samples > MAX_SAMPLES
            || samples != budget.sample_counts[track_index]
            || chunks.offsets.is_empty()
            || runs.is_empty()
            || runs[0].first_chunk != 1
            || (sizes.sample_size == 0 && sizes.sample_sizes.len() != samples as usize)
        {
            return Err(invalid());
        }
        for (i, run) in runs.iter().enumerate() {
            if run.first_chunk == 0
                || run.first_chunk as usize > chunks.offsets.len()
                || run.samples_per_chunk == 0
                || run.samples_per_chunk > MAX_SAMPLES
                || run.sample_description_index != 1
                || (i > 0 && runs[i - 1].first_chunk >= run.first_chunk)
            {
                return Err(invalid());
            }
        }
        let (mut run, mut sample) = (0, 0usize);
        for (index, &offset) in chunks.offsets.iter().enumerate() {
            while run + 1 < runs.len() && runs[run + 1].first_chunk as usize <= index + 1 {
                run += 1;
            }
            let mut length = 0u64;
            for _ in 0..runs[run].samples_per_chunk {
                if sample >= samples as usize {
                    return Err(invalid());
                }
                let n = if sizes.sample_size == 0 {
                    sizes.sample_sizes[sample]
                } else {
                    sizes.sample_size
                };
                length = length.checked_add(u64::from(n)).ok_or_else(invalid)?;
                sample += 1;
            }
            let end = offset.checked_add(length).ok_or_else(invalid)?;
            if length == 0
                || !budget
                    .mdat
                    .iter()
                    .any(|r| offset >= r.start && end <= r.end)
            {
                return Err(invalid());
            }
        }
        if sample != samples as usize {
            return Err(invalid());
        }
    }
    let mut info = video.ok_or_else(invalid)?;
    info.audio = audio;
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;
    const OWNED: &[u8] = include_bytes!("../../../tests/fixtures/owned-video.mp4");
    #[test]
    fn owned_hevc_receive_preserves_sample_bounds_and_send_contract() {
        let bytes = include_bytes!("../../../tests/fixtures/owned-video-hevc.mp4");
        let info = inspect_mp4(bytes).unwrap();
        assert_eq!(
            (info.width, info.height, info.duration_ms, info.audio),
            (32, 32, 1000, true)
        );
        assert!(inspect_send_mp4("owned.mp4", bytes).is_err());
        let config = bytes.windows(4).position(|b| b == b"hvcC").unwrap() + 4;
        for (offset, value) in [(0, 0), (22, 255)] {
            let mut broken = bytes.to_vec();
            broken[config + offset] = value;
            assert!(inspect_mp4(&broken).is_err());
        }
    }
    fn changed(name: &[u8; 4], delta: usize, value: &[u8]) -> Vec<u8> {
        let mut bytes = OWNED.to_vec();
        // avc1 also appears in ftyp's brand list; mutate the sample entry.
        let at = if name == b"avc1" {
            bytes.windows(4).rposition(|b| b == name)
        } else {
            bytes.windows(4).position(|b| b == name)
        }
        .unwrap()
            + delta;
        bytes[at..at + value.len()].copy_from_slice(value);
        bytes
    }
    #[test]
    fn owned_h264_aac_container_has_bounded_dimensions_and_duration() {
        let info = inspect_mp4(OWNED).unwrap();
        assert_eq!(
            (info.width, info.height, info.duration_ms, info.audio),
            (32, 32, 200, true)
        );
    }
    #[test]
    fn truncated_boxes_and_over_limit_input_are_rejected() {
        for n in [0, 7, 16, OWNED.len() - 1] {
            assert!(inspect_mp4(&OWNED[..n]).is_err());
        }
        let mut appended = OWNED.to_vec();
        appended.push(0);
        assert!(inspect_mp4(&appended).is_err());
        assert!(inspect_mp4(&vec![0; MAX_VIDEO_BYTES + 1]).is_err());
    }
    #[test]
    fn table_allocation_counts_are_rejected_before_parser_allocation() {
        for name in [b"stco", b"stss", b"stsc", b"stts", b"stsd"] {
            assert!(inspect_mp4(&changed(name, 8, &u32::MAX.to_be_bytes())).is_err());
        }
        assert!(inspect_mp4(&changed(b"stsz", 12, &u32::MAX.to_be_bytes())).is_err());
    }
    #[test]
    fn external_offsets_unsupported_codec_and_invalid_geometry_are_rejected() {
        for bytes in [
            changed(b"stco", 12, &0u32.to_be_bytes()),
            changed(b"avc1", 0, b"hvc1"),
            changed(b"avc1", 28, &4097u16.to_be_bytes()),
            changed(b"mdhd", 16, &0u32.to_be_bytes()),
            changed(b"dref", 20, &0u32.to_be_bytes()),
        ] {
            assert!(inspect_mp4(&bytes).is_err());
        }
    }
}
