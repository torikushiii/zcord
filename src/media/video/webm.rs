use anyhow::{Context, Result, ensure};

const EBML: u64 = 0x1A45_DFA3;
const SEGMENT: u64 = 0x1853_8067;
const SEEK_HEAD: u64 = 0x114D_9B74;
const CUES: u64 = 0x1C53_BB6B;
const CHAPTERS: u64 = 0x1043_A770;
const TAGS: u64 = 0x1254_C367;
const ATTACHMENTS: u64 = 0x1941_A469;
const INFO: u64 = 0x1549_A966;
const TIMESTAMP_SCALE: u64 = 0x2A_D7B1;
const TRACKS: u64 = 0x1654_AE6B;
const TRACK_ENTRY: u64 = 0xAE;
const TRACK_NUMBER: u64 = 0xD7;
const CODEC_ID: u64 = 0x86;
const DEFAULT_DURATION: u64 = 0x23_E383;
const VIDEO: u64 = 0xE0;
const PIXEL_WIDTH: u64 = 0xB0;
const PIXEL_HEIGHT: u64 = 0xBA;
const COLOUR: u64 = 0x55B0;
const MATRIX: u64 = 0x55B1;
const RANGE: u64 = 0x55B9;
const CLUSTER: u64 = 0x1F43_B675;
const CLUSTER_TIMESTAMP: u64 = 0xE7;
const SIMPLE_BLOCK: u64 = 0xA3;
const BLOCK_GROUP: u64 = 0xA0;
const BLOCK: u64 = 0xA1;
const BLOCK_ADDITIONS: u64 = 0x75A1;
const BLOCK_MORE: u64 = 0xA6;
const BLOCK_ADD_ID: u64 = 0xEE;
const BLOCK_ADDITIONAL: u64 = 0xA5;
const MAX_FRAMES: usize = 4096;
const MAX_UNKNOWN_DEPTH: usize = 4;
const SEGMENT_CHILDREN: [u64; 8] = [
    SEEK_HEAD,
    INFO,
    TRACKS,
    CLUSTER,
    CUES,
    CHAPTERS,
    TAGS,
    ATTACHMENTS,
];
const MAX_SIDE: u32 = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Matrix {
    Bt601,
    Bt709,
}

pub(crate) struct Frame {
    pub(crate) time: u64,
    pub(crate) color: Vec<u8>,
    pub(crate) alpha: Option<Vec<u8>>,
}

pub(crate) struct Track {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) matrix: Matrix,
    pub(crate) full_range: bool,
    pub(crate) frame_duration: Option<u64>,
    pub(crate) frames: Vec<Frame>,
}

impl Track {
    pub(crate) fn has_alpha(&self) -> bool {
        self.frames.iter().any(|frame| frame.alpha.is_some())
    }

    pub(crate) fn frame_nanos(&self) -> Option<u64> {
        self.frame_duration.filter(|nanos| *nanos > 0).or_else(|| {
            let first = self.frames.first()?.time;
            let last = self.frames.last()?.time;
            let gaps = self.frames.len().checked_sub(1).filter(|gaps| *gaps > 0)? as u64;
            Some((last.saturating_sub(first) / gaps).max(1))
        })
    }
}

pub(crate) fn is_webm(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0x1A, 0x45, 0xDF, 0xA3])
}

struct Element<'a> {
    id: u64,
    body: &'a [u8],
}

fn vint(data: &[u8], pos: &mut usize, keep_marker: bool) -> Option<(u64, bool)> {
    let first = *data.get(*pos)?;
    if first == 0 {
        return None;
    }
    let len = first.leading_zeros() as usize + 1;
    let end = pos.checked_add(len).filter(|end| *end <= data.len())?;
    let mask = (1u64 << (8 - len)) - 1;
    let lead = u64::from(first) & mask;
    let mut value = if keep_marker { u64::from(first) } else { lead };
    let mut unknown = !keep_marker && lead == mask;
    for byte in &data[*pos + 1..end] {
        value = (value << 8) | u64::from(*byte);
        unknown &= *byte == 0xFF;
    }
    *pos = end;
    Some((value, unknown))
}

fn known_end(data: &[u8], pos: usize, size: u64) -> usize {
    usize::try_from(size)
        .ok()
        .and_then(|size| pos.checked_add(size))
        .map_or(data.len(), |end| end.min(data.len()))
}

fn ends_unknown(parent: u64, id: u64) -> bool {
    id == EBML || id == SEGMENT || (parent != SEGMENT && SEGMENT_CHILDREN.contains(&id))
}

// Ending an unknown-size element at its parent's end would swallow the sibling Clusters after it.
fn unknown_end(data: &[u8], mut pos: usize, parent: u64, depth: usize) -> usize {
    while pos < data.len() {
        let start = pos;
        let Some((id, _)) = vint(data, &mut pos, true) else {
            return data.len();
        };
        if ends_unknown(parent, id) {
            return start;
        }
        let Some((size, unknown)) = vint(data, &mut pos, false) else {
            return data.len();
        };
        pos = match (unknown, depth < MAX_UNKNOWN_DEPTH) {
            (false, _) => known_end(data, pos, size),
            (true, true) => unknown_end(data, pos, id, depth + 1),
            (true, false) => return data.len(),
        };
    }
    data.len()
}

fn elements(data: &[u8]) -> Vec<Element<'_>> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < data.len() {
        let Some((id, _)) = vint(data, &mut pos, true) else {
            break;
        };
        let Some((size, unknown)) = vint(data, &mut pos, false) else {
            break;
        };
        let end = if unknown {
            unknown_end(data, pos, id, 0)
        } else {
            known_end(data, pos, size)
        };
        out.push(Element {
            id,
            body: &data[pos..end],
        });
        pos = end;
    }
    out
}

fn uint(body: &[u8]) -> u64 {
    body.iter()
        .take(8)
        .fold(0, |value, byte| (value << 8) | u64::from(*byte))
}

fn block(body: &[u8], track: u64) -> Option<(i16, &[u8])> {
    let mut pos = 0;
    let (number, _) = vint(body, &mut pos, false)?;
    let relative = i16::from_be_bytes([*body.get(pos)?, *body.get(pos + 1)?]);
    let flags = *body.get(pos + 2)?;
    (number == track && flags & 0x06 == 0).then(|| (relative, &body[pos + 3..]))
}

struct Demux {
    track: u64,
    scale: u64,
    frames: Vec<Frame>,
}

impl Demux {
    fn push(&mut self, base: u64, relative: i16, color: &[u8], alpha: Option<Vec<u8>>) {
        if self.frames.len() >= MAX_FRAMES || color.is_empty() {
            return;
        }
        let ticks = (base as i64).saturating_add(i64::from(relative)).max(0) as u64;
        self.frames.push(Frame {
            time: ticks.saturating_mul(self.scale),
            color: color.to_vec(),
            alpha,
        });
    }

    fn cluster(&mut self, body: &[u8]) {
        let mut base = 0;
        for child in elements(body) {
            match child.id {
                CLUSTER_TIMESTAMP => base = uint(child.body),
                SIMPLE_BLOCK => {
                    if let Some((relative, color)) = block(child.body, self.track) {
                        self.push(base, relative, color, None);
                    }
                }
                BLOCK_GROUP => self.group(child.body, base),
                _ => {}
            }
        }
    }

    fn group(&mut self, body: &[u8], base: u64) {
        let mut frame = None;
        let mut alpha = None;
        for part in elements(body) {
            match part.id {
                BLOCK => frame = block(part.body, self.track),
                BLOCK_ADDITIONS => {
                    for more in elements(part.body)
                        .into_iter()
                        .filter(|more| more.id == BLOCK_MORE)
                    {
                        let mut id = 1;
                        let mut data = None;
                        for field in elements(more.body) {
                            match field.id {
                                BLOCK_ADD_ID => id = uint(field.body),
                                BLOCK_ADDITIONAL => data = Some(field.body),
                                _ => {}
                            }
                        }
                        if id == 1
                            && let Some(data) = data.filter(|data| !data.is_empty())
                        {
                            alpha = Some(data.to_vec());
                        }
                    }
                }
                _ => {}
            }
        }
        if let Some((relative, color)) = frame {
            self.push(base, relative, color, alpha);
        }
    }
}

struct VideoTrack {
    number: u64,
    width: u32,
    height: u32,
    matrix: Matrix,
    full_range: bool,
    frame_duration: Option<u64>,
}

fn track_entry(body: &[u8]) -> Option<VideoTrack> {
    let mut number = 0;
    let mut vp9 = false;
    let mut width = 0;
    let mut height = 0;
    let mut matrix = Matrix::Bt601;
    let mut full_range = false;
    let mut frame_duration = None;
    for field in elements(body) {
        match field.id {
            TRACK_NUMBER => number = uint(field.body),
            CODEC_ID => vp9 = field.body == b"V_VP9",
            DEFAULT_DURATION => frame_duration = Some(uint(field.body)),
            VIDEO => {
                for video in elements(field.body) {
                    match video.id {
                        PIXEL_WIDTH => width = u32::try_from(uint(video.body)).unwrap_or(0),
                        PIXEL_HEIGHT => height = u32::try_from(uint(video.body)).unwrap_or(0),
                        COLOUR => {
                            for colour in elements(video.body) {
                                match colour.id {
                                    MATRIX if uint(colour.body) == 1 => matrix = Matrix::Bt709,
                                    RANGE => full_range = uint(colour.body) == 2,
                                    _ => {}
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    (vp9 && number > 0).then_some(VideoTrack {
        number,
        width,
        height,
        matrix,
        full_range,
        frame_duration,
    })
}

pub(crate) fn demux(data: &[u8]) -> Result<Track> {
    ensure!(is_webm(data), "not a WebM file");
    let segment = elements(data)
        .into_iter()
        .find(|element| element.id == SEGMENT)
        .context("WebM file has no segment")?
        .body;
    let mut scale = 1_000_000;
    let mut video: Option<VideoTrack> = None;
    let mut demux: Option<Demux> = None;
    for element in elements(segment) {
        match element.id {
            INFO => {
                for field in elements(element.body) {
                    if field.id == TIMESTAMP_SCALE {
                        scale = uint(field.body).max(1);
                    }
                }
            }
            TRACKS if video.is_none() => {
                video = elements(element.body)
                    .into_iter()
                    .filter(|entry| entry.id == TRACK_ENTRY)
                    .find_map(|entry| track_entry(entry.body));
            }
            CLUSTER => {
                let Some(video) = &video else {
                    continue;
                };
                demux
                    .get_or_insert_with(|| Demux {
                        track: video.number,
                        scale,
                        frames: Vec::new(),
                    })
                    .cluster(element.body);
            }
            _ => {}
        }
    }
    let video = video.context("WebM file has no VP9 track")?;
    ensure!(
        (1..=MAX_SIDE).contains(&video.width) && (1..=MAX_SIDE).contains(&video.height),
        "unsupported WebM dimensions"
    );
    let frames = demux.map(|demux| demux.frames).unwrap_or_default();
    ensure!(!frames.is_empty(), "WebM file has no frames");
    Ok(Track {
        width: video.width,
        height: video.height,
        matrix: video.matrix,
        full_range: video.full_range,
        frame_duration: video.frame_duration,
        frames,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn element(id: &[u8], body: &[u8]) -> Vec<u8> {
        assert!(body.len() < 0x7F);
        let mut out = id.to_vec();
        out.push(0x80 | body.len() as u8);
        out.extend_from_slice(body);
        out
    }

    fn concat(parts: &[Vec<u8>]) -> Vec<u8> {
        parts.concat()
    }

    fn block_body(track: u8, relative: i16, payload: &[u8]) -> Vec<u8> {
        let mut out = vec![0x80 | track];
        out.extend_from_slice(&relative.to_be_bytes());
        out.push(0x80);
        out.extend_from_slice(payload);
        out
    }

    fn file() -> Vec<u8> {
        let group = element(
            &[0xA0],
            &concat(&[
                element(&[0xA1], &block_body(1, 33, b"color-2")),
                element(
                    &[0x75, 0xA1],
                    &element(
                        &[0xA6],
                        &concat(&[element(&[0xEE], &[0x01]), element(&[0xA5], b"alpha-2")]),
                    ),
                ),
            ]),
        );
        let cluster = element(
            &[0x1F, 0x43, 0xB6, 0x75],
            &concat(&[
                element(&[0xE7], &[0x00]),
                element(&[0xA3], &block_body(1, 0, b"color-1")),
                element(&[0xA3], &block_body(2, 0, b"other-track")),
                group,
            ]),
        );
        file_with(cluster)
    }

    fn file_with(clusters: Vec<u8>) -> Vec<u8> {
        let header = element(&[0x1A, 0x45, 0xDF, 0xA3], &element(&[0x42, 0x82], b"webm"));
        let info = element(
            &[0x15, 0x49, 0xA9, 0x66],
            &element(&[0x2A, 0xD7, 0xB1], &[0x0F, 0x42, 0x40]),
        );
        let video = element(
            &[0xE0],
            &concat(&[
                element(&[0xB0], &[0x04, 0x00]),
                element(&[0xBA], &[0x02, 0x00]),
                element(&[0x53, 0xC0], &[0x01]),
                element(&[0x55, 0xB0], &element(&[0x55, 0xB1], &[0x01])),
            ]),
        );
        let entry = element(
            &[0xAE],
            &concat(&[
                element(&[0xD7], &[0x01]),
                element(&[0x86], b"V_VP9"),
                element(&[0x23, 0xE3, 0x83], &[0x01, 0xFC, 0xA0, 0x55]),
                video,
            ]),
        );
        let tracks = element(&[0x16, 0x54, 0xAE, 0x6B], &entry);
        let mut segment_body = concat(&[info, tracks, clusters]);
        let mut segment = vec![
            0x18, 0x53, 0x80, 0x67, 0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
        ];
        segment.append(&mut segment_body);
        concat(&[header, segment])
    }

    #[test]
    fn demux_reads_vp9_frames_with_alpha_additions() {
        let track = demux(&file()).unwrap();
        assert_eq!((track.width, track.height), (1024, 512));
        assert_eq!(track.matrix, Matrix::Bt709);
        assert!(!track.full_range);
        assert_eq!(track.frames.len(), 2);
        assert_eq!(track.frames[0].color, b"color-1");
        assert_eq!(track.frames[0].alpha, None);
        assert_eq!(track.frames[1].time, 33_000_000);
        assert_eq!(track.frames[1].alpha.as_deref(), Some(&b"alpha-2"[..]));
        assert!(track.has_alpha());
        assert_eq!(track.frame_nanos(), Some(33_333_333));
    }

    #[test]
    fn demux_reads_every_unknown_size_sibling_cluster() {
        let clusters: Vec<Vec<u8>> = (0..6u8)
            .map(|index| {
                let mut cluster = vec![0x1F, 0x43, 0xB6, 0x75, 0xFF];
                cluster.extend(element(&[0xE7], &[index * 10]));
                cluster.extend(element(&[0xA3], &block_body(1, 0, &[b'c', index])));
                cluster
            })
            .collect();
        let track = demux(&file_with(concat(&clusters))).unwrap();
        assert_eq!(track.frames.len(), 6);
        for (index, frame) in track.frames.iter().enumerate() {
            assert_eq!(frame.color, [b'c', index as u8]);
            assert_eq!(frame.time, index as u64 * 10_000_000);
        }
    }

    #[test]
    fn demux_rejects_other_data() {
        assert!(demux(b"\x00\x00\x00\x18ftypmp42").is_err());
        let mut truncated = file();
        truncated.truncate(40);
        assert!(demux(&truncated).is_err());
    }
}
