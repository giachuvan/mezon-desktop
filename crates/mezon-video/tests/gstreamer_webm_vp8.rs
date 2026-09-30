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
    if let Some(ticks) = demuxer.info().duration() {
        let header_seconds = ticks * timestamp_scale as f64 / 1_000_000_000.0;
        let wrong_divide_by_1e9 = ticks / 1_000_000_000.0;
        assert!(
            header_seconds > 5.0,
            "header duration ticks={ticks} scale={timestamp_scale} => {header_seconds}s"
        );
        assert!(
            wrong_divide_by_1e9 < 0.05,
            "ticks are NOT nanoseconds; dividing by 1e9 wrongly yields {wrong_divide_by_1e9}"
        );
    }
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

#[test]
fn decode_many_vp8_frames_from_fixture() {
    use oxideav_vp8::state::Vp8DecoderState;
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
    let mut vp8 = Vp8DecoderState::new();
    let mut frame = matroska_demuxer::Frame::default();
    let mut ok = 0u32;
    let mut err = 0u32;
    let mut skipped = 0u32;
    let mut log = String::new();
    while demuxer.next_frame(&mut frame).ok() == Some(true) {
        if frame.track != video_track {
            skipped += 1;
            continue;
        }
        match vp8.decode_frame(&frame.data) {
            Ok(decoded) => {
                ok += 1;
                if ok <= 3 || ok % 30 == 0 {
                    log.push_str(&format!(
                        "ok frame#{ok} ts={} {}x{} bytes={} shown={:?}\n",
                        frame.timestamp,
                        decoded.width,
                        decoded.height,
                        frame.data.len(),
                        vp8.last_frame_shown()
                    ));
                }
            }
            Err(e) => {
                err += 1;
                if err <= 8 {
                    log.push_str(&format!(
                        "err frame ts={} bytes={} err={e:?}\n",
                        frame.timestamp,
                        frame.data.len()
                    ));
                }
            }
        }
    }
    log.push_str(&format!(
        "summary ok={ok} err={err} skipped_non_video={skipped}\n"
    ));
    std::fs::write("/tmp/webm_decode_probe.txt", &log).ok();
    assert!(
        ok > 10,
        "expected many decoded frames, got ok={ok} err={err}; {log}"
    );
}
