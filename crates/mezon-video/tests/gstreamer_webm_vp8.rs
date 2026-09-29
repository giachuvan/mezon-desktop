use std::io::Cursor;
use std::path::Path;

use matroska_demuxer::{MatroskaFile, TrackType};

fn matroska_codec_id(id: &str) -> &str {
    id.trim_end_matches('\0')
}

fn is_vp8_video_track(track: &matroska_demuxer::TrackEntry) -> bool {
    track.track_type() == TrackType::Video && matroska_codec_id(track.codec_id()) == "V_VP8"
}

fn fixture_path() -> Option<std::path::PathBuf> {
    if let Ok(path) = std::env::var("MEZON_TEST_GSTREAMER_WEBM") {
        let path = Path::new(path.as_str());
        if path.is_file() {
            return Some(path.to_path_buf());
        }
    }
    let path = Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../target/test-screencast.webm"
    ));
    path.is_file().then(|| path.to_path_buf())
}

#[test]
fn gstreamer_muxed_webm_codec_id_carries_null_suffix() {
    let Some(path) = fixture_path() else {
        return;
    };
    let bytes = std::fs::read(path).expect("read fixture");
    let demuxer = MatroskaFile::open(Cursor::new(bytes)).expect("open demuxer");
    let video = demuxer
        .tracks()
        .iter()
        .find(|track| track.track_type() == TrackType::Video)
        .expect("video track");
    assert_eq!(video.codec_id(), "V_VP8\0");
    assert!(is_vp8_video_track(video));
}

#[test]
fn gstreamer_muxed_webm_duration_from_frame_timestamps() {
    let Some(path) = fixture_path() else {
        return;
    };
    let bytes = std::fs::read(path).expect("read fixture");
    let mut demuxer = MatroskaFile::open(Cursor::new(bytes)).expect("open demuxer");
    let video_track = demuxer
        .tracks()
        .iter()
        .find(|track| is_vp8_video_track(track))
        .expect("video track")
        .track_number()
        .get();
    let timestamp_scale = demuxer.info().timestamp_scale().get();
    let mut frame = matroska_demuxer::Frame::default();
    let mut max_ns = 0u64;
    while demuxer.next_frame(&mut frame).ok() == Some(true) {
        if frame.track == video_track {
            max_ns = max_ns.max(frame.timestamp.saturating_mul(timestamp_scale));
        }
    }
    let duration = max_ns as f64 / 1_000_000_000.0;
    assert!(duration > 5.0);
    assert!(duration < 7.0);
}
