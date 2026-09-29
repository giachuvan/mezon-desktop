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
