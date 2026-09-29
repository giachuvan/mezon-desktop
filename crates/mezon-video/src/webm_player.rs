use std::cell::{Cell, RefCell};
use std::io::{Cursor, Read};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use matroska_demuxer::{DemuxError, Frame, MatroskaFile, TrackType};
use oxideav_vp8::state::Vp8DecoderState;
use parking_lot::Mutex;

use crate::{PlayerError, VideoFrame, VideoProbe};

const MAX_WEBM_BYTES: usize = 64 * 1024 * 1024;

pub(crate) fn is_webm_source(url: &str) -> bool {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    path.rsplit('/')
        .next()
        .is_some_and(|name| name.to_ascii_lowercase().ends_with(".webm"))
}

struct DemuxState {
    demuxer: MatroskaFile<Cursor<Vec<u8>>>,
    video_track: u64,
    timestamp_scale: u64,
    vp8: Vp8DecoderState,
    last_frame_ns: u64,
    cached: Option<VideoFrame>,
    eos: bool,
}

pub struct WebmPlayerImpl {
    state: Mutex<DemuxState>,
    duration_seconds: f64,
    playing: Cell<bool>,
    play_started_at: RefCell<Option<Instant>>,
    play_offset_ns: Cell<u64>,
    volume: Cell<f32>,
    muted: Cell<bool>,
    failed: AtomicBool,
    max_size: Option<(u32, u32)>,
}

impl WebmPlayerImpl {
    pub fn open(url: &str, max_size: Option<(u32, u32)>) -> Result<Self, PlayerError> {
        if url.is_empty() {
            return Err(PlayerError::InvalidUrl);
        }
        let bytes = load_bytes(url)?;
        let cursor = Cursor::new(bytes);
        let demuxer = MatroskaFile::open(cursor).map_err(|error| {
            tracing::warn!(target: "mezon_video", ?error, "webm demuxer open failed");
            PlayerError::Open
        })?;
        let video_track = demuxer
            .tracks()
            .iter()
            .find(|track| track.track_type() == TrackType::Video && track.codec_id() == "V_VP8")
            .map(|track| track.track_number().get())
            .ok_or_else(|| {
                tracing::warn!(target: "mezon_video", "webm has no VP8 video track");
                PlayerError::Open
            })?;
        let timestamp_scale = demuxer.info().timestamp_scale().get();
        let duration_seconds = demuxer
            .info()
            .duration()
            .map(|ns| ns / 1_000_000_000.0)
            .filter(|value| value.is_finite() && *value > 0.0)
            .unwrap_or(0.0);
        let mut state = DemuxState {
            demuxer,
            video_track,
            timestamp_scale,
            vp8: Vp8DecoderState::new(),
            last_frame_ns: 0,
            cached: None,
            eos: false,
        };
        if !decode_until(&mut state, 0, max_size)? {
            return Err(PlayerError::Open);
        }
        Ok(Self {
            state: Mutex::new(state),
            duration_seconds,
            playing: Cell::new(false),
            play_started_at: RefCell::new(None),
            play_offset_ns: Cell::new(0),
            volume: Cell::new(1.0),
            muted: Cell::new(false),
            failed: AtomicBool::new(false),
            max_size,
        })
    }

    pub fn copy_frame(&self) -> Option<VideoFrame> {
        if self.failed.load(Ordering::SeqCst) {
            return None;
        }
        let target_ns = if self.playing.get() {
            self.play_offset_ns.get().saturating_add(self.elapsed_ns())
        } else {
            self.play_offset_ns.get()
        };
        let mut state = self.state.lock();
        if self.playing.get() && !state.eos {
            let _ = advance_to(&mut state, target_ns, self.max_size);
        }
        state.cached.clone()
    }

    pub fn play(&self) {
        if self.failed.load(Ordering::SeqCst) {
            return;
        }
        if !self.playing.replace(true) {
            *self.play_started_at.borrow_mut() = Some(Instant::now());
        }
    }

    pub fn pause(&self) {
        if self.playing.replace(false) {
            self.play_offset_ns
                .set(self.play_offset_ns.get().saturating_add(self.elapsed_ns()));
            *self.play_started_at.borrow_mut() = None;
        }
    }

    pub fn is_playing(&self) -> bool {
        self.playing.get()
    }

    pub fn current_time(&self) -> f64 {
        let ns = if self.playing.get() {
            self.play_offset_ns.get().saturating_add(self.elapsed_ns())
        } else {
            self.play_offset_ns.get()
        };
        (ns as f64 / 1_000_000_000.0).min(self.duration_seconds)
    }

    pub fn duration(&self) -> f64 {
        self.duration_seconds
    }

    pub fn seek(&self, to_seconds: f64) {
        if self.failed.load(Ordering::SeqCst) {
            return;
        }
        let target = if to_seconds.is_finite() && to_seconds >= 0.0 {
            to_seconds
        } else {
            0.0
        };
        let target_ns = (target * 1_000_000_000.0) as u64;
        self.playing.set(false);
        *self.play_started_at.borrow_mut() = None;
        self.play_offset_ns.set(target_ns);
        let mut state = self.state.lock();
        state.vp8 = Vp8DecoderState::new();
        state.last_frame_ns = 0;
        state.cached = None;
        state.eos = false;
        let seek_ts = target_ns / state.timestamp_scale.max(1);
        if state.demuxer.seek(seek_ts).is_err() {
            self.failed.store(true, Ordering::SeqCst);
            return;
        }
        if advance_to(&mut state, target_ns, self.max_size).is_err() {
            self.failed.store(true, Ordering::SeqCst);
        }
    }

    pub fn set_volume(&self, volume: f32) {
        self.volume.set(volume.clamp(0.0, 1.0));
    }

    pub fn volume(&self) -> f32 {
        self.volume.get()
    }

    pub fn set_muted(&self, muted: bool) {
        self.muted.set(muted);
    }

    pub fn is_muted(&self) -> bool {
        self.muted.get()
    }

    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::SeqCst)
    }

    fn elapsed_ns(&self) -> u64 {
        self.play_started_at
            .borrow()
            .as_ref()
            .map(|started| started.elapsed().as_nanos() as u64)
            .unwrap_or(0)
    }
}

pub fn probe_webm(path: &str, max_poster_edge: u32) -> Option<VideoProbe> {
    let bytes = load_bytes(path).ok()?;
    let mut demuxer = MatroskaFile::open(Cursor::new(bytes)).ok()?;
    let video_track = demuxer
        .tracks()
        .iter()
        .find(|track| track.track_type() == TrackType::Video && track.codec_id() == "V_VP8")?
        .track_number()
        .get();
    let mut frame = Frame::default();
    while demuxer.next_frame(&mut frame).ok()? {
        if frame.track != video_track {
            continue;
        }
        let decoded = oxideav_vp8::state::Vp8DecoderState::new()
            .decode_frame(&frame.data)
            .ok()?;
        #[cfg(windows)]
        let poster_jpeg = {
            let bgra = crate::frame_util::i420_to_bgra(
                decoded.width,
                decoded.height,
                &decoded.y,
                &decoded.u,
                &decoded.v,
            )?;
            crate::poster::encode_poster_jpeg(
                &bgra,
                decoded.width,
                decoded.height,
                (decoded.width as usize).saturating_mul(4),
                false,
                crate::poster::Turn::default(),
                max_poster_edge,
            )
        };
        return Some(VideoProbe {
            width: decoded.width,
            height: decoded.height,
            #[cfg(windows)]
            poster_jpeg,
            #[cfg(target_os = "macos")]
            poster_jpeg: None,
        });
    }
    None
}

fn load_bytes(url: &str) -> Result<Vec<u8>, PlayerError> {
    if url.starts_with("http://") || url.starts_with("https://") {
        let mut response = ureq::get(url).call().map_err(|error| {
            tracing::warn!(target: "mezon_video", ?error, "webm download failed");
            PlayerError::Open
        })?;
        let mut body = Vec::new();
        response
            .body_mut()
            .with_config()
            .limit(MAX_WEBM_BYTES)
            .read_to_end(&mut body)
            .map_err(|_| PlayerError::Open)?;
        Ok(body)
    } else if let Some(path) = url.strip_prefix("file://") {
        let bytes = std::fs::read(path).map_err(|_| PlayerError::Open)?;
        if bytes.len() > MAX_WEBM_BYTES {
            return Err(PlayerError::Open);
        }
        Ok(bytes)
    } else {
        let bytes = std::fs::read(url).map_err(|_| PlayerError::Open)?;
        if bytes.len() > MAX_WEBM_BYTES {
            return Err(PlayerError::Open);
        }
        Ok(bytes)
    }
}

fn advance_to(
    state: &mut DemuxState,
    target_ns: u64,
    max_size: Option<(u32, u32)>,
) -> Result<bool, DemuxError> {
    if state.eos {
        return Ok(true);
    }
    if state.last_frame_ns > target_ns {
        let seek_ts = target_ns / state.timestamp_scale.max(1);
        state.demuxer.seek(seek_ts)?;
        state.vp8 = Vp8DecoderState::new();
        state.last_frame_ns = 0;
        state.cached = None;
        state.eos = false;
    }
    while state.last_frame_ns <= target_ns {
        if !decode_next_video_frame(state, max_size)? {
            state.eos = true;
            break;
        }
    }
    Ok(state.cached.is_some())
}

fn decode_until(
    state: &mut DemuxState,
    target_ns: u64,
    max_size: Option<(u32, u32)>,
) -> Result<bool, PlayerError> {
    advance_to(state, target_ns, max_size).map_err(|error| {
        tracing::warn!(target: "mezon_video", ?error, "webm decode failed");
        PlayerError::Open
    })
}

fn decode_next_video_frame(
    state: &mut DemuxState,
    max_size: Option<(u32, u32)>,
) -> Result<bool, DemuxError> {
    let mut frame = Frame::default();
    while state.demuxer.next_frame(&mut frame)? {
        if frame.track != state.video_track {
            continue;
        }
        let timestamp_ns = frame.timestamp.saturating_mul(state.timestamp_scale);
        let decoded = match state.vp8.decode_frame(&frame.data) {
            Ok(decoded) => decoded,
            Err(_) => continue,
        };
        if let Some(video_frame) = vp8_to_frame(&decoded, max_size) {
            state.last_frame_ns = timestamp_ns;
            state.cached = Some(video_frame);
            return Ok(true);
        }
    }
    Ok(false)
}

fn vp8_to_frame(
    decoded: &oxideav_vp8::decoder::Vp8DecodedFrame,
    max_size: Option<(u32, u32)>,
) -> Option<VideoFrame> {
    #[cfg(target_os = "macos")]
    {
        return crate::webm_frame_macos::pixel_buffer_from_vp8(decoded, max_size);
    }
    #[cfg(windows)]
    {
        let mut bgra = crate::frame_util::i420_to_bgra(
            decoded.width,
            decoded.height,
            &decoded.y,
            &decoded.u,
            &decoded.v,
        )?;
        let (width, height) = match max_size {
            Some((max_w, max_h)) if max_w > 0 && max_h > 0 => {
                let max_w = max_w.min(decoded.width);
                let max_h = max_h.min(decoded.height);
                if decoded.width <= max_w && decoded.height <= max_h {
                    (decoded.width, decoded.height)
                } else {
                    let scale = (max_w as f32 / decoded.width as f32)
                        .min(max_h as f32 / decoded.height as f32);
                    let out_w = ((decoded.width as f32 * scale).round() as u32).max(1);
                    let out_h = ((decoded.height as f32 * scale).round() as u32).max(1);
                    bgra = scale_bgra(&bgra, decoded.width, decoded.height, out_w, out_h)?;
                    (out_w, out_h)
                }
            }
            _ => (decoded.width, decoded.height),
        };
        crate::render_frame::bgra_to_frame(width, height, bgra)
    }
}

#[cfg(windows)]
fn scale_bgra(source: &[u8], src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> Option<Vec<u8>> {
    let src_w = src_w as usize;
    let src_h = src_h as usize;
    let dst_w = dst_w as usize;
    let dst_h = dst_h as usize;
    let src_stride = src_w.checked_mul(4)?;
    if source.len() < src_stride.checked_mul(src_h)? {
        return None;
    }
    let mut out = vec![0u8; dst_w.checked_mul(dst_h)?.checked_mul(4)?];
    for y in 0..dst_h {
        let src_y = y * src_h / dst_h;
        for x in 0..dst_w {
            let src_x = x * src_w / dst_w;
            let from = src_y * src_stride + src_x * 4;
            let to = y * dst_w * 4 + x * 4;
            out[to..to + 4].copy_from_slice(&source[from..from + 4]);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webm_sources_are_detected_by_extension() {
        assert!(is_webm_source("https://cdn.example/clip.webm"));
        assert!(is_webm_source("https://cdn.example/clip.webm?token=1"));
        assert!(!is_webm_source("https://cdn.example/clip.mp4"));
    }
}
