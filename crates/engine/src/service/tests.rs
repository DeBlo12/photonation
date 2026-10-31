// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Behaviour of the background workers, driven by synthetic photos.

use super::*;
use photonation_image::{Orientation, Rect};
use photonation_raw::RawError;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Barrier, Mutex};
use std::time::{Duration, Instant};

/// Generous, so slow CI machines never flake; tests normally finish in milliseconds.
const TIMEOUT: Duration = Duration::from_secs(30);
const PHOTO_SIZE: (usize, usize) = (96, 64);

/// Serves synthetic photos by file name: `broken*` fails to decode,
/// `panic*` panics inside the decoder, `slow*` waits at two barriers (one
/// to say it started, one to be released), and `big*` is large enough to
/// be scaled down for a thumbnail.
#[derive(Default)]
struct FakeDecoder {
    decodes: Mutex<HashMap<PathBuf, usize>>,
    slow_gate: Option<(Arc<Barrier>, Arc<Barrier>)>,
}

impl FakeDecoder {
    fn decodes(&self, path: &Path) -> usize {
        self.decodes
            .lock()
            .expect("decode counts")
            .get(path)
            .copied()
            .unwrap_or(0)
    }
}

impl Decoder for FakeDecoder {
    fn decode(&self, path: &Path) -> Result<SensorImage, RawError> {
        *self
            .decodes
            .lock()
            .expect("decode counts")
            .entry(path.into())
            .or_default() += 1;
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if name.starts_with("broken") {
            return Err(RawError::Decode {
                path: path.into(),
                message: "not a raw file".into(),
            });
        }
        if name.starts_with("panic") {
            panic!("decoder bug triggered by {name}");
        }
        if name.starts_with("slow")
            && let Some((started, release)) = &self.slow_gate
        {
            started.wait();
            release.wait();
        }
        let (width, height) = if name.starts_with("big") {
            (800, 600)
        } else {
            PHOTO_SIZE
        };
        Ok(SensorImage::test_pattern(
            width,
            height,
            Orientation::Normal,
        ))
    }

    /// `jpeg*` files carry a grey 30×20 preview.
    fn embedded_preview(&self, path: &Path, _long_edge: usize) -> Option<RenderedImage> {
        let name = path.file_name()?.to_string_lossy();
        name.starts_with("jpeg").then(|| RenderedImage {
            width: 30,
            height: 20,
            rgb: vec![128; 30 * 20 * 3],
            histogram: Histogram::default(),
        })
    }

    fn capture_time(&self, _path: &Path) -> Option<String> {
        Some("2026:06:20 18:02:23".into())
    }
}

struct Harness {
    service: RenderService,
    decoder: Arc<FakeDecoder>,
    notifications: Arc<AtomicUsize>,
}

fn harness(decoder: FakeDecoder) -> Harness {
    let decoder = Arc::new(decoder);
    let notifications = Arc::new(AtomicUsize::new(0));
    let counter = notifications.clone();
    let service = RenderService::with_decoder(decoder.clone(), move || {
        counter.fetch_add(1, Ordering::SeqCst);
    });
    Harness {
        service,
        decoder,
        notifications,
    }
}

impl Harness {
    fn next_event(&self) -> Event {
        self.service
            .events
            .recv_timeout(TIMEOUT)
            .expect("an event before the timeout")
    }

    /// Events up to and including the first one `last` accepts.
    fn events_until(&self, mut last: impl FnMut(&Event) -> bool) -> Vec<Event> {
        let mut events = Vec::new();
        loop {
            let event = self.next_event();
            let done = last(&event);
            events.push(event);
            if done {
                return events;
            }
        }
    }

    fn preview(&self, path: &str, generation: u64, kind: PreviewKind, long_edge: usize) {
        self.service.request_preview(PreviewRequest {
            path: path.into(),
            settings: Settings::default(),
            long_edge: Some(long_edge),
            generation,
            kind,
            point_color_overlay: None,
        });
    }
}

fn is_preview(event: &Event) -> bool {
    matches!(event, Event::Preview { .. })
}

fn is_error_for(event: &Event, expected: &str) -> bool {
    matches!(event, Event::Error { path: Some(path), .. } if path == Path::new(expected))
}

#[test]
fn preview_announces_the_photo_then_renders_it() {
    let h = harness(FakeDecoder::default());
    h.preview("a.cr3", 7, PreviewKind::Edited, 48);
    let events = h.events_until(is_preview);

    let Event::PhotoLoaded(info) = &events[0] else {
        panic!("expected PhotoLoaded first, got {:?}", events[0]);
    };
    assert_eq!(info.path, Path::new("a.cr3"));
    let crop = Rect::new(2, 2, PHOTO_SIZE.0 - 4, PHOTO_SIZE.1 - 4);
    assert_eq!(info.size, (crop.w, crop.h));
    assert!(info.as_shot.0.is_finite() && info.as_shot.1.is_finite());

    let Some(Event::Preview {
        path,
        generation,
        kind,
        image,
    }) = events.last()
    else {
        unreachable!("events_until stops at a preview")
    };
    assert_eq!(
        (path.as_path(), *generation, *kind),
        (Path::new("a.cr3"), 7, PreviewKind::Edited)
    );
    let sensor = SensorImage::test_pattern(PHOTO_SIZE.0, PHOTO_SIZE.1, Orientation::Normal);
    let expected = photonation_pipeline::output_size(&sensor, &Settings::default(), Some(48));
    assert_eq!((image.width, image.height), expected);
    assert_eq!(image.rgb.len(), image.width * image.height * 3);
    let counted: u32 = image.histogram.luminance.iter().sum();
    assert_eq!(counted as usize, image.width * image.height);
}

#[test]
fn preview_keeps_the_current_photo_decoded() {
    let h = harness(FakeDecoder::default());
    for generation in 1..=3 {
        h.preview("a.cr3", generation, PreviewKind::Edited, 48);
        h.events_until(is_preview);
    }
    assert_eq!(h.decoder.decodes(Path::new("a.cr3")), 1);

    // Switching photos decodes the new one; coming back decodes again,
    // because only the current photo stays in memory.
    h.preview("b.cr3", 4, PreviewKind::Edited, 48);
    h.events_until(is_preview);
    h.preview("a.cr3", 5, PreviewKind::Edited, 48);
    h.events_until(is_preview);
    assert_eq!(h.decoder.decodes(Path::new("b.cr3")), 1);
    assert_eq!(h.decoder.decodes(Path::new("a.cr3")), 2);
}

#[test]
fn a_burst_of_requests_renders_only_the_newest_of_each_kind() {
    let (started, release) = (Arc::new(Barrier::new(2)), Arc::new(Barrier::new(2)));
    let h = harness(FakeDecoder {
        slow_gate: Some((started.clone(), release.clone())),
        ..FakeDecoder::default()
    });

    // Keep the worker busy decoding the first request…
    h.preview("slow.cr3", 1, PreviewKind::Edited, 48);
    started.wait();
    // …while a slider drag queues more requests, plus an eyedropper pick.
    for generation in 2..=6 {
        h.preview("slow.cr3", generation, PreviewKind::Edited, 48);
    }
    h.preview("slow.cr3", 6, PreviewKind::Before, 48);
    h.service
        .pick_white_balance("slow.cr3".into(), Settings::default(), [0.5, 0.5]);
    release.wait();

    let events = h.events_until(|e| {
        matches!(
            e,
            Event::Preview {
                kind: PreviewKind::Before,
                ..
            }
        )
    });
    let edited: Vec<u64> = events
        .iter()
        .filter_map(|e| match e {
            Event::Preview {
                generation,
                kind: PreviewKind::Edited,
                ..
            } => Some(*generation),
            _ => None,
        })
        .collect();
    assert_eq!(edited, vec![1, 6], "intermediate generations are skipped");
    let picks = events
        .iter()
        .filter(|e| matches!(e, Event::WhiteBalancePicked { .. }))
        .count();
    assert_eq!(picks, 1, "eyedropper picks are never dropped");
    assert_eq!(h.decoder.decodes(Path::new("slow.cr3")), 1);
}

#[test]
fn eyedropper_reports_a_white_balance() {
    let h = harness(FakeDecoder::default());
    h.service
        .pick_white_balance("a.cr3".into(), Settings::default(), [0.5, 0.5]);
    let events = h.events_until(|e| matches!(e, Event::WhiteBalancePicked { .. }));
    let Some(Event::WhiteBalancePicked {
        path,
        temperature,
        tint,
    }) = events.last()
    else {
        unreachable!()
    };
    assert_eq!(path, Path::new("a.cr3"));
    assert!((2000.0..25000.0).contains(temperature), "{temperature}");
    assert!(tint.is_finite());
}

#[test]
fn color_picker_reports_a_color() {
    let h = harness(FakeDecoder::default());
    h.service
        .pick_color("a.cr3".into(), Settings::default(), [0.5, 0.5]);
    let events = h.events_until(|e| matches!(e, Event::ColorPicked { .. }));
    let Some(Event::ColorPicked { path, color }) = events.last() else {
        unreachable!()
    };
    assert_eq!(path, Path::new("a.cr3"));
    assert!((0.0..=1.0).contains(&color.lightness), "{color:?}");
    assert!((0.0..360.0).contains(&color.hue), "{color:?}");
}

#[test]
fn decode_errors_are_reported_and_the_worker_carries_on() {
    let h = harness(FakeDecoder::default());
    h.preview("broken.cr3", 1, PreviewKind::Edited, 48);
    let events = h.events_until(|e| matches!(e, Event::Error { .. }));
    let Some(Event::Error { path, message }) = events.last() else {
        unreachable!()
    };
    assert_eq!(path.as_deref(), Some(Path::new("broken.cr3")));
    assert!(message.contains("not a raw file"), "{message}");

    h.preview("a.cr3", 2, PreviewKind::Edited, 48);
    h.events_until(is_preview);
}

#[test]
fn a_panicking_decoder_does_not_kill_the_preview_worker() {
    let h = harness(FakeDecoder::default());
    h.preview("panic.cr3", 1, PreviewKind::Edited, 48);
    let events = h.events_until(|e| is_error_for(e, "panic.cr3"));
    let Some(Event::Error { message, .. }) = events.last() else {
        unreachable!()
    };
    assert!(message.contains("internal error"), "{message}");

    h.preview("a.cr3", 2, PreviewKind::Edited, 48);
    h.events_until(is_preview);
}

#[test]
fn thumbnails_are_small_renders() {
    let h = harness(FakeDecoder::default());
    h.service
        .request_thumbnail("big.cr3".into(), Settings::default());
    let events = h.events_until(|e| matches!(e, Event::Thumbnail { .. }));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::CaptureTime { captured, .. }
            if captured == "2026:06:20 18:02:23")),
        "{events:?}"
    );
    let Some(Event::Thumbnail {
        path,
        image,
        rendered,
    }) = events.last()
    else {
        unreachable!()
    };
    assert_eq!(path, Path::new("big.cr3"));
    assert!(rendered, "no embedded preview to show first");
    assert_eq!(
        image.width.max(image.height),
        crate::thumbnails::THUMBNAIL_LONG_EDGE
    );
}

#[test]
fn embedded_previews_show_until_the_render_arrives() {
    let h = harness(FakeDecoder::default());
    h.service
        .request_thumbnail("jpeg.cr3".into(), Settings::default());
    let thumbnails: Vec<(usize, bool)> = (0..2)
        .map(|_| {
            let events = h.events_until(|e| matches!(e, Event::Thumbnail { .. }));
            let Some(Event::Thumbnail {
                image, rendered, ..
            }) = events.last()
            else {
                unreachable!()
            };
            (image.width, *rendered)
        })
        .collect();
    assert_eq!(thumbnails[0], (30, false), "the embedded preview first");
    assert!(thumbnails[1].1, "then the render");
}

#[test]
fn a_refresh_overtakes_an_older_render() {
    let h = harness(FakeDecoder::default());
    let edited = Settings {
        exposure: 1.0,
        ..Settings::default()
    };
    h.service
        .request_thumbnail("a.cr3".into(), Settings::default());
    h.service.refresh_thumbnail("a.cr3".into(), edited);
    let events = h.events_until(|e| matches!(e, Event::Thumbnail { .. }));
    assert!(matches!(
        events.last(),
        Some(Event::Thumbnail { rendered: true, .. })
    ));
    // Only the newest settings render; nothing else is coming.
    assert!(
        h.service
            .events
            .recv_timeout(Duration::from_millis(300))
            .is_err()
    );
}

#[test]
fn survey_images_use_the_embedded_preview_of_unedited_photos() {
    let h = harness(FakeDecoder::default());
    h.service
        .request_survey_image("jpeg.cr3".into(), Settings::default(), 1600);
    let Event::SurveyImage { image, .. } = h.next_event() else {
        panic!("a survey image");
    };
    assert_eq!(image.width, 30);
    let edited = Settings {
        exposure: 1.0,
        ..Settings::default()
    };
    h.service
        .request_survey_image("jpeg.cr3".into(), edited, 1600);
    let events = h.events_until(|e| matches!(e, Event::SurveyImage { .. }));
    let Some(Event::SurveyImage { image, .. }) = events.last() else {
        unreachable!()
    };
    assert_ne!(image.width, 30, "edited photos are rendered");
}

#[test]
fn thumbnail_workers_survive_more_panics_than_there_are_workers() {
    let h = harness(FakeDecoder::default());
    let failing = crate::thumbnails::THUMBNAIL_WORKERS + 1;
    for index in 0..failing {
        h.service
            .request_thumbnail(format!("panic-{index}.cr3").into(), Settings::default());
    }
    h.service
        .request_thumbnail("a.cr3".into(), Settings::default());

    let mut errors = 0;
    let mut thumbnail = false;
    while errors < failing || !thumbnail {
        match h.next_event() {
            Event::Error { .. } => errors += 1,
            Event::Thumbnail { path, .. } => thumbnail = path == Path::new("a.cr3"),
            Event::CaptureTime { .. } => {}
            other => panic!("unexpected {other:?}"),
        }
    }
}

#[test]
fn export_writes_each_photo_and_reports_progress_and_failures() {
    let photos_dir = tempfile::tempdir().expect("temp dir");
    let out = tempfile::tempdir().expect("temp dir");
    let a = photos_dir.path().join("a.cr3");
    let broken = photos_dir.path().join("broken.cr3");
    let b = photos_dir.path().join("b.cr3");
    let h = harness(FakeDecoder::default());
    let options = ExportOptions {
        long_edge: Some(64),
        ..ExportOptions::default()
    };
    h.service.export(
        vec![
            (a.clone(), None), // settings from the (missing) sidecar
            (broken.clone(), None),
            (b.clone(), Some(Settings::default())),
        ],
        out.path().into(),
        options,
    );

    let events = h.events_until(|e| matches!(e, Event::ExportFinished { .. }));
    let progress: Vec<(usize, usize)> = events
        .iter()
        .filter_map(|e| match e {
            Event::ExportProgress { done, total } => Some((*done, *total)),
            _ => None,
        })
        .collect();
    assert_eq!(progress, vec![(0, 3), (1, 3), (2, 3)]);
    let Some(Event::ExportFinished { exported, failures }) = events.last() else {
        unreachable!()
    };
    assert_eq!(*exported, 2);
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].0, broken);

    for name in ["a.jpg", "b.jpg"] {
        let bytes = std::fs::read(out.path().join(name)).expect("exported file");
        assert_eq!(&bytes[..2], &[0xFF, 0xD8], "{name} is a JPEG");
    }
    // No temporary files and nothing for the failed photo.
    assert_eq!(std::fs::read_dir(out.path()).expect("list").count(), 2);
}

#[test]
fn export_never_replaces_existing_files() {
    let out = tempfile::tempdir().expect("temp dir");
    let earlier = out.path().join("a.jpg");
    std::fs::write(&earlier, "an earlier export").expect("write");
    let h = harness(FakeDecoder::default());
    let options = ExportOptions {
        long_edge: Some(32),
        ..ExportOptions::default()
    };
    // Two photos with the same stem (e.g. a CR3 and its DNG) as well.
    h.service.export(
        vec![
            ("a.cr3".into(), Some(Settings::default())),
            ("a.dng".into(), Some(Settings::default())),
        ],
        out.path().into(),
        options,
    );
    let events = h.events_until(|e| matches!(e, Event::ExportFinished { .. }));
    assert!(matches!(
        events.last(),
        Some(Event::ExportFinished { exported: 2, failures }) if failures.is_empty()
    ));
    assert_eq!(
        std::fs::read_to_string(&earlier).expect("read"),
        "an earlier export"
    );
    assert!(out.path().join("a-2.jpg").is_file());
    assert!(out.path().join("a-3.jpg").is_file());
}

#[test]
fn a_panicking_export_is_a_failure_not_a_dead_worker() {
    let out = tempfile::tempdir().expect("temp dir");
    let h = harness(FakeDecoder::default());
    for batch in [vec!["panic.cr3", "a.cr3"], vec!["b.cr3"]] {
        let photos = batch
            .into_iter()
            .map(|name| (PathBuf::from(name), Some(Settings::default())))
            .collect();
        h.service.export(
            photos,
            out.path().into(),
            ExportOptions {
                long_edge: Some(32),
                ..ExportOptions::default()
            },
        );
    }
    let finished: Vec<(usize, usize)> = (0..2)
        .map(|_| {
            match h
                .events_until(|e| matches!(e, Event::ExportFinished { .. }))
                .pop()
            {
                Some(Event::ExportFinished { exported, failures }) => (exported, failures.len()),
                _ => unreachable!(),
            }
        })
        .collect();
    assert_eq!(finished, vec![(1, 1), (1, 0)]);
}

#[test]
fn every_event_wakes_the_front_end() {
    let h = harness(FakeDecoder::default());
    h.preview("a.cr3", 1, PreviewKind::Edited, 48);
    let received = h.events_until(is_preview).len();
    // `notify` runs right after each send, so allow it a moment to catch up.
    let deadline = Instant::now() + TIMEOUT;
    while h.notifications.load(Ordering::SeqCst) < received && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert_eq!(h.notifications.load(Ordering::SeqCst), received);
}

#[test]
fn panic_messages_keep_their_detail() {
    let from_str: Box<dyn Any + Send> = Box::new("index out of bounds");
    let from_string: Box<dyn Any + Send> = Box::new(String::from("bad crop"));
    let opaque: Box<dyn Any + Send> = Box::new(42);
    assert!(panic_message(&*from_str).contains("index out of bounds"));
    assert!(panic_message(&*from_string).contains("bad crop"));
    assert!(panic_message(&*opaque).contains("unknown cause"));
}

#[test]
fn removals_are_filled_in_the_background_then_rendered() {
    use photonation_develop::{BrushStroke, Removal, RemovalMethod};
    let h = harness(FakeDecoder::default());
    let settings = Settings {
        removals: vec![Removal::new(
            RemovalMethod::ContentAware,
            BrushStroke {
                points: vec![[0.5, 0.5]],
                radius: 0.08,
            },
        )],
        ..Settings::default()
    };
    let request = |generation| PreviewRequest {
        path: "a.cr3".into(),
        settings: settings.clone(),
        long_edge: None,
        generation,
        kind: PreviewKind::Edited,
        point_color_overlay: None,
    };
    h.service.request_preview(request(1));
    let events = h.events_until(|e| matches!(e, Event::RemovalsFilled { .. }));
    let Some(Event::Preview {
        image: unfilled, ..
    }) = events.iter().find(|e| is_preview(e))
    else {
        panic!("the first preview does not wait for the fill: {events:?}")
    };
    let unfilled = unfilled.rgb.clone();
    h.service.request_preview(request(2));
    let events = h.events_until(is_preview);
    let Some(Event::Preview {
        generation: 2,
        image,
        ..
    }) = events.last()
    else {
        panic!("{events:?}")
    };
    assert_ne!(image.rgb, unfilled, "the second preview shows the fill");
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, Event::RemovalsFilled { .. })),
        "the fill is cached, not recomputed"
    );
}
