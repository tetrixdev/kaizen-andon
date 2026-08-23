//! Local activity capture: evidence for the minutes the ledger cannot explain.
//!
//! The ledger's blind spot is everything with no AI session and no entry in the
//! other system: mail, calls, browser work, meetings. This fills it the way
//! ManicTime does, with a continuous record reviewed rather than remembered.
//!
//! Three rules shape the whole design.
//!
//! **It never leaves the machine.** Nothing here uploads. What reaches an AI is
//! whatever the operator hands it at checkpoint time, deliberately, one bucket
//! at a time.
//!
//! **The platform surface is as thin as it can be made.** There is no Rust on
//! the box this is written on, and `check.sh` type-checks in a LINUX container,
//! which does not look inside `#[cfg(windows)]` at all. So everything that can
//! be decided without Win32 is decided here and tested here: bucketing, naming,
//! compositing, encoding, dedup, the disk floor and the archive layout. The
//! platform module answers two questions only, and `stub.rs` answers them with
//! invented data so this module is exercised on Linux exactly as it runs on
//! Windows.
//!
//! **The disk floor is not pruning.** Retention is deliberately never automatic
//! here; the operator deletes. But refusing to be the process that takes a box
//! to 100% is a different promise, and this one has already been broken twice.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

#[cfg(windows)]
#[path = "win.rs"]
mod platform;
#[cfg(not(windows))]
#[path = "stub.rs"]
mod platform;

pub use platform::Screens;

/// Minutes per archive. A file per minute is ~480 files a day, which Explorer
/// and every AV scanner in the world will crawl over; fifteen gives 32 files a
/// day and puts the index in the filename.
pub const BUCKET_MINUTES: u32 = 15;

/// Widest composite kept at full size.
///
/// Effectively no downscale: measured on a real 1440p desktop, a full-size
/// frame is 238KB at this quality and 183KB once deflated, so a working day of
/// two screens costs well under 200MB. Half-size saved two thirds of that and
/// was the plan until the numbers arrived, at which point it was paying with
/// legibility for a saving nothing needed. The cap exists only so an absurd
/// wall of monitors cannot produce a single enormous frame.
pub const MAX_WIDTH: u32 = 8000;

/// JPEG quality.
///
/// 70 rather than 60 at full size. The saving between them is 24KB a frame,
/// around 11MB across a day, and what it buys back is ringing around glyphs and
/// UI edges, which is the one artifact that hurts a picture kept solely so text
/// can be recognised later.
pub const QUALITY: u8 = 70;

/// What one monitor looked like, straight from the platform.
///
/// `x` and `y` are the monitor's own position on the virtual desktop, which is
/// what lets a window rectangle be mapped into the composite. Without them a
/// secret can be located on the desktop and still not be found in the picture.
pub struct Shot {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// A window worth hiding, in virtual-desktop coordinates.
#[derive(Debug, Clone, PartialEq)]
pub struct Secret {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// What the machine was doing this minute, apart from what it looked like.
#[derive(Debug, Clone, PartialEq)]
pub struct Probe {
    pub idle_secs: u64,
    pub locked: bool,
    pub process: String,
    pub title: String,
}

/// The two questions only the platform can answer.
pub trait Source {
    fn probe(&self) -> Result<Probe, String>;
    fn shots(&self) -> Result<Vec<Shot>, String>;
    /// Every visible window belonging to an excluded process, wherever it is.
    ///
    /// Deliberately NOT "the foreground window, if excluded". A password
    /// manager sitting open on the second screen while the browser has focus is
    /// exactly the case that matters, and a foreground test misses all of it.
    fn secrets(&self, excluded: &[String]) -> Vec<Secret>;
    /// Free bytes on the volume holding `path`.
    fn free_bytes(&self, path: &Path) -> u64;
}

/// What a tick did, so the tray can say something true about it.
#[derive(Debug, Clone, PartialEq)]
pub enum Tick {
    /// Outside every window that earns capture, or paused.
    Resting,
    /// Stopped because the volume is under the floor. Deletes nothing.
    NoRoom { free_mb: u64 },
    /// A frame was stored.
    Kept { bytes: usize },
    /// The screen had not changed since the previous minute.
    Unchanged,
    /// Locked workstation: the line is written, the picture is not.
    Away,
}

/// A quarter hour of frames, held in memory until it is sealed into a zip.
struct Bucket {
    day: String,
    start: String,
    frames: Vec<(String, Vec<u8>)>,
    lines: Vec<String>,
}

pub struct Recorder<S: Source> {
    source: S,
    root: PathBuf,
    floor_bytes: u64,
    excluded: Vec<String>,
    bucket: Option<Bucket>,
    last_hash: Option<[u8; 32]>,
}

impl<S: Source> Recorder<S> {
    pub fn new(source: S, root: PathBuf, floor_bytes: u64, excluded: Vec<String>) -> Self {
        Self {
            source,
            root,
            floor_bytes,
            excluded,
            bucket: None,
            last_hash: None,
        }
    }

    /// One minute. `stamp` is the operator's wall clock, already local, as
    /// `("2026-08-20", "0914")`; `capturing` is Kaizen's answer to whether this
    /// minute is inside a context that earns it, which is deliberately NOT
    /// recomputed here. The widget must not carry a second opinion about what a
    /// working day is.
    pub fn tick(&mut self, stamp: (&str, &str), capturing: bool) -> Result<Tick, String> {
        if !capturing {
            self.seal()?;
            return Ok(Tick::Resting);
        }

        let free = self.source.free_bytes(&self.root);
        if free < self.floor_bytes {
            self.seal()?;
            return Ok(Tick::NoRoom {
                free_mb: free / 1_048_576,
            });
        }

        let probe = self.source.probe()?;

        if probe.locked {
            self.push(stamp, &probe, "locked", None)?;
            return Ok(Tick::Away);
        }

        let secrets = self.source.secrets(&self.excluded);
        let frame = encode(self.source.shots()?, &secrets)?;
        let hash: [u8; 32] = Sha256::digest(&frame).into();

        // Reading one long document is thirty identical screenshots. Storing
        // the first and pointing at it costs a line instead of a file. The hash
        // is taken AFTER blanking, so two minutes differing only inside a
        // hidden rectangle are correctly one frame.
        if self.last_hash == Some(hash) {
            self.push(stamp, &probe, "same", None)?;
            return Ok(Tick::Unchanged);
        }

        let bytes = frame.len();
        self.last_hash = Some(hash);
        self.push(stamp, &probe, "kept", Some(frame))?;
        Ok(Tick::Kept { bytes })
    }

    fn push(
        &mut self,
        (day, minute): (&str, &str),
        probe: &Probe,
        note: &str,
        frame: Option<Vec<u8>>,
    ) -> Result<(), String> {
        let start = bucket_start(minute);

        match &self.bucket {
            Some(open) if open.day == day && open.start == start => {}
            // A new quarter, or midnight. Either way the open one is finished.
            _ => {
                self.seal()?;
                self.bucket = Some(Bucket {
                    day: day.to_string(),
                    start,
                    frames: Vec::new(),
                    lines: Vec::new(),
                });
            }
        }

        let line = format!(
            "{}\t{}\t{}\t{}\t{}",
            minute,
            probe.idle_secs,
            note,
            probe.process,
            probe.title.replace(['\t', '\n', '\r'], " ")
        );

        // The day's TSV is appended live and never sealed, so a checkpoint can
        // read the day, or grep it, without unzipping anything at all.
        let dir = self.root.join(day);
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join(format!("{day}.tsv")))
            .map_err(|e| e.to_string())?;
        writeln!(f, "{line}").map_err(|e| e.to_string())?;

        let bucket = self.bucket.as_mut().expect("just opened");
        bucket.lines.push(line);
        if let Some(frame) = frame {
            bucket.frames.push((format!("{minute}.jpg"), frame));
        }
        Ok(())
    }

    /// Close the open bucket into its zip. Written to `.part` and renamed, so a
    /// crash mid-write cannot leave a torn archive and a reader never opens a
    /// half-written one.
    pub fn seal(&mut self) -> Result<(), String> {
        let Some(bucket) = self.bucket.take() else {
            return Ok(());
        };
        if bucket.lines.is_empty() {
            return Ok(());
        }

        let dir = self.root.join(&bucket.day);
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

        let name = zip_name(&bucket.day, &bucket.start);
        let part = dir.join(format!("{name}.part"));
        let mut zip = zip::ZipWriter::new(File::create(&part).map_err(|e| e.to_string())?);

        // Deflate the frames as well as the text, which is NOT the obvious
        // call and was got wrong here first. "JPEG is already compressed" is
        // true of the pixels and false of the file: an encoder shipping the
        // standard Huffman tables rather than optimised ones leaves real
        // redundancy in the entropy-coded stream, and deflate takes it.
        // Measured on a screenshot re-encoded at this exact quality it took
        // another 35% off, and MORE at lower quality rather than less.
        //
        // The asymmetry settles it rather than that number, which varies by
        // encoder: the downside is bounded at roughly no saving for one pass
        // per quarter hour, and the upside is a third of the archive.
        let squeezed = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);

        zip.start_file("titles.tsv", squeezed)
            .map_err(|e| e.to_string())?;
        zip.write_all(bucket.lines.join("\n").as_bytes())
            .map_err(|e| e.to_string())?;

        for (name, bytes) in &bucket.frames {
            zip.start_file(name, squeezed).map_err(|e| e.to_string())?;
            zip.write_all(bytes).map_err(|e| e.to_string())?;
        }

        zip.finish().map_err(|e| e.to_string())?;
        fs::rename(&part, dir.join(&name)).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Replace the exclusion list, which the tray can change while this runs.
    pub fn set_excluded(&mut self, excluded: Vec<String>) {
        self.excluded = excluded;
    }

    /// Total bytes held under the archive root, for the tray to show. Nothing
    /// prunes; seeing it grow is the whole point.
    pub fn size_bytes(&self) -> u64 {
        walk(&self.root)
    }
}

/// Floor a minute to its quarter: 0914 -> 0900, 0915 -> 0915.
fn bucket_start(minute: &str) -> String {
    let (h, m) = minute.split_at(2);
    let m: u32 = m.parse().unwrap_or(0);
    format!("{h}{:02}", (m / BUCKET_MINUTES) * BUCKET_MINUTES)
}

/// `2026-08-20_0900-0915.zip`. The span is in the name so the directory sorts
/// into a timeline and needs no index beside it.
fn zip_name(day: &str, start: &str) -> String {
    let (h, m) = start.split_at(2);
    let end = h.parse::<u32>().unwrap_or(0) * 60 + m.parse::<u32>().unwrap_or(0) + BUCKET_MINUTES;
    format!("{day}_{start}-{:02}{:02}.zip", (end / 60) % 24, end % 60)
}

/// Every monitor laid out left to right in one image, scaled down and encoded.
/// One file for the whole desk: two files for two screens means guessing later
/// which one was which.
///
/// `secrets` are blanked BEFORE the image is scaled or encoded, so no buffer
/// holding those pixels is ever written anywhere. Blanking rather than skipping
/// the whole frame is the point: the rest of the minute is still evidence, and
/// a black rectangle says honestly that something was hidden there.
fn encode(shots: Vec<Shot>, secrets: &[Secret]) -> Result<Vec<u8>, String> {
    let canvas = composite(shots, secrets)?;
    let (width, height) = (canvas.width(), canvas.height());

    let scaled = if width > MAX_WIDTH {
        let h = (height as f32 * (MAX_WIDTH as f32 / width as f32))
            .round()
            .max(1.0) as u32;
        image::imageops::resize(&canvas, MAX_WIDTH, h, image::imageops::FilterType::Triangle)
    } else {
        canvas
    };

    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, QUALITY)
        .encode_image(&image::DynamicImage::ImageRgba8(scaled).to_rgb8())
        .map_err(|e| e.to_string())?;
    Ok(out)
}

/// The desk as one image, at full size, with every secret already blacked out.
///
/// Split from `encode` so a test can look at the pixels: whether a rectangle
/// landed on the right screen is not a question a JPEG can answer, and getting
/// it wrong is worse than not blanking at all because it still looks blanked.
fn composite(shots: Vec<Shot>, secrets: &[Secret]) -> Result<image::RgbaImage, String> {
    if shots.is_empty() {
        return Err("no monitors".into());
    }

    let width: u32 = shots.iter().map(|s| s.width).sum();
    let height: u32 = shots.iter().map(|s| s.height).max().unwrap_or(0);
    if width == 0 || height == 0 {
        return Err("a monitor reported no size".into());
    }

    let mut canvas = image::RgbaImage::new(width, height);
    let mut x = 0;
    for shot in &shots {
        let tile = image::RgbaImage::from_raw(shot.width, shot.height, shot.rgba.clone())
            .ok_or("a monitor's pixels did not match its size")?;
        image::imageops::replace(&mut canvas, &tile, x as i64, 0);

        // The composite is monitors packed left to right, which is NOT the
        // desktop's own coordinate space: a screen at desktop x=-1920 sits at
        // composite x=0. Every rectangle has to make that trip or the blanking
        // lands on the wrong screen, which is worse than not blanking at all
        // because it looks like it worked.
        for secret in secrets {
            let left = secret.left.max(shot.x) - shot.x + x as i32;
            let top = secret.top.max(shot.y) - shot.y;
            let right = secret.right.min(shot.x + shot.width as i32) - shot.x + x as i32;
            let bottom = secret.bottom.min(shot.y + shot.height as i32) - shot.y;

            for py in top.max(0)..bottom.min(height as i32) {
                for px in left.max(0)..right.min((x + shot.width) as i32) {
                    canvas.put_pixel(px as u32, py as u32, image::Rgba([0, 0, 0, 255]));
                }
            }
        }

        x += shot.width;
    }

    Ok(canvas)
}

fn walk(dir: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => walk(&e.path()),
            Ok(_) => e.metadata().map(|m| m.len()).unwrap_or(0),
            Err(_) => 0,
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicU32, Ordering};

    static N: AtomicU32 = AtomicU32::new(0);

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "kaizen-capture-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    struct Fake {
        probe: RefCell<Probe>,
        pixel: RefCell<u8>,
        free: RefCell<u64>,
        secrets: RefCell<Vec<Secret>>,
    }

    impl Fake {
        fn new() -> Self {
            Self {
                probe: RefCell::new(Probe {
                    idle_secs: 0,
                    locked: false,
                    process: "OUTLOOK.EXE".into(),
                    title: "Inbox".into(),
                }),
                pixel: RefCell::new(1),
                free: RefCell::new(u64::MAX),
                secrets: RefCell::new(Vec::new()),
            }
        }
    }

    impl Source for Fake {
        fn probe(&self) -> Result<Probe, String> {
            Ok(self.probe.borrow().clone())
        }

        fn shots(&self) -> Result<Vec<Shot>, String> {
            let v = *self.pixel.borrow();
            Ok(vec![
                Shot {
                    x: 0,
                    y: 0,
                    width: 8,
                    height: 4,
                    rgba: vec![v; 8 * 4 * 4],
                },
                Shot {
                    x: 8,
                    y: 0,
                    width: 8,
                    height: 4,
                    rgba: vec![v; 8 * 4 * 4],
                },
            ])
        }

        fn secrets(&self, _excluded: &[String]) -> Vec<Secret> {
            self.secrets.borrow().clone()
        }

        fn free_bytes(&self, _path: &Path) -> u64 {
            *self.free.borrow()
        }
    }

    #[test]
    fn a_minute_floors_to_its_quarter() {
        assert_eq!(bucket_start("0900"), "0900");
        assert_eq!(bucket_start("0914"), "0900");
        assert_eq!(bucket_start("0915"), "0915");
        assert_eq!(bucket_start("0959"), "0945");
        assert_eq!(bucket_start("0000"), "0000");
    }

    #[test]
    fn the_name_carries_the_span_it_covers() {
        assert_eq!(zip_name("2026-08-20", "0900"), "2026-08-20_0900-0915.zip");
        assert_eq!(zip_name("2026-08-20", "0945"), "2026-08-20_0945-1000.zip");
        // The last quarter of the day wraps rather than reading 2400.
        assert_eq!(zip_name("2026-08-20", "2345"), "2026-08-20_2345-0000.zip");
    }

    #[test]
    fn monitors_are_laid_out_side_by_side_in_one_image() {
        let wide = encode(
            vec![
                Shot {
                    x: 0,
                    y: 0,
                    width: 8,
                    height: 4,
                    rgba: vec![9; 8 * 4 * 4],
                },
                Shot {
                    x: 8,
                    y: 0,
                    width: 8,
                    height: 4,
                    rgba: vec![9; 8 * 4 * 4],
                },
            ],
            &[],
        )
        .unwrap();

        // A JPEG, and one image rather than two files to reconcile later.
        assert_eq!(&wide[..2], &[0xFF, 0xD8]);
        assert!(!wide.is_empty());
    }

    #[test]
    fn a_monitor_whose_pixels_do_not_match_its_size_is_refused() {
        let err = encode(
            vec![Shot {
                x: 0,
                y: 0,
                width: 8,
                height: 4,
                rgba: vec![0; 3],
            }],
            &[],
        )
        .unwrap_err();
        assert!(err.contains("did not match"), "{err}");
    }

    #[test]
    fn an_unchanged_screen_costs_a_line_rather_than_a_file() {
        let mut rec = Recorder::new(Fake::new(), scratch(), 0, vec![]);

        assert!(matches!(
            rec.tick(("2026-08-20", "0900"), true).unwrap(),
            Tick::Kept { .. }
        ));
        assert_eq!(
            rec.tick(("2026-08-20", "0901"), true).unwrap(),
            Tick::Unchanged
        );

        *rec.source.pixel.borrow_mut() = 200; // the screen changed
        assert!(matches!(
            rec.tick(("2026-08-20", "0902"), true).unwrap(),
            Tick::Kept { .. }
        ));
    }

    #[test]
    fn a_locked_workstation_is_recorded_but_not_pictured() {
        let mut rec = Recorder::new(Fake::new(), scratch(), 0, vec![]);
        rec.source.probe.borrow_mut().locked = true;

        assert_eq!(rec.tick(("2026-08-20", "0900"), true).unwrap(), Tick::Away);
        assert!(rec.bucket.as_ref().unwrap().frames.is_empty());
        assert_eq!(rec.bucket.as_ref().unwrap().lines.len(), 1);
    }

    #[test]
    fn resting_seals_what_is_open_rather_than_holding_it() {
        let root = scratch();
        let mut rec = Recorder::new(Fake::new(), root.clone(), 0, vec![]);

        rec.tick(("2026-08-20", "0900"), true).unwrap();
        assert_eq!(
            rec.tick(("2026-08-20", "0901"), false).unwrap(),
            Tick::Resting
        );

        assert!(root.join("2026-08-20/2026-08-20_0900-0915.zip").exists());
        assert!(rec.bucket.is_none());
    }

    #[test]
    fn the_floor_stops_capture_without_deleting_anything() {
        let root = scratch();
        let mut rec = Recorder::new(Fake::new(), root.clone(), 1_000_000, vec![]);
        rec.tick(("2026-08-20", "0900"), true).unwrap();

        *rec.source.free.borrow_mut() = 5;
        let tick = rec.tick(("2026-08-20", "0901"), true).unwrap();

        assert!(matches!(tick, Tick::NoRoom { .. }), "{tick:?}");
        // What was already captured is sealed, not discarded.
        assert!(root.join("2026-08-20/2026-08-20_0900-0915.zip").exists());
    }

    #[test]
    fn crossing_the_quarter_seals_the_one_behind_it() {
        let root = scratch();
        let mut rec = Recorder::new(Fake::new(), root.clone(), 0, vec![]);

        rec.tick(("2026-08-20", "0914"), true).unwrap();
        *rec.source.pixel.borrow_mut() = 50;
        rec.tick(("2026-08-20", "0915"), true).unwrap();

        assert!(root.join("2026-08-20/2026-08-20_0900-0915.zip").exists());
        assert_eq!(rec.bucket.as_ref().unwrap().start, "0915");
    }

    #[test]
    fn the_days_tsv_is_readable_without_opening_a_single_zip() {
        let root = scratch();
        let mut rec = Recorder::new(Fake::new(), root.clone(), 0, vec![]);

        rec.source.probe.borrow_mut().title = "Ticket ZP0138157\twith a tab".into();
        rec.tick(("2026-08-20", "0900"), true).unwrap();
        rec.tick(("2026-08-20", "0901"), true).unwrap();

        let tsv = fs::read_to_string(root.join("2026-08-20/2026-08-20.tsv")).unwrap();
        let lines: Vec<_> = tsv.lines().collect();

        assert_eq!(lines.len(), 2);
        assert!(
            lines[0].starts_with("0900\t0\tkept\tOUTLOOK.EXE\t"),
            "{}",
            lines[0]
        );
        assert!(lines[1].contains("\tsame\t"), "{}", lines[1]);
        // A tab inside a title would otherwise invent a sixth column.
        assert_eq!(lines[0].matches('\t').count(), 4, "{}", lines[0]);
    }

    #[test]
    fn a_sealed_quarter_stands_on_its_own() {
        let root = scratch();
        let mut rec = Recorder::new(Fake::new(), root.clone(), 0, vec![]);

        rec.tick(("2026-08-20", "0900"), true).unwrap();
        *rec.source.pixel.borrow_mut() = 77;
        rec.tick(("2026-08-20", "0901"), true).unwrap();
        rec.seal().unwrap();

        let path = root.join("2026-08-20/2026-08-20_0900-0915.zip");
        let mut zip = zip::ZipArchive::new(File::open(&path).unwrap()).unwrap();
        let names: Vec<String> = zip.file_names().map(String::from).collect();

        assert!(names.contains(&"titles.tsv".to_string()), "{names:?}");
        assert!(names.contains(&"0900.jpg".to_string()), "{names:?}");
        assert!(names.contains(&"0901.jpg".to_string()), "{names:?}");
        // Nothing is left half written beside it.
        assert!(!root
            .join("2026-08-20/2026-08-20_0900-0915.zip.part")
            .exists());
        assert!(zip.by_name("titles.tsv").is_ok());
    }

    /// Two 8x4 screens side by side, the second at desktop x=8.
    fn desk(v: u8) -> Vec<Shot> {
        vec![
            Shot {
                x: 0,
                y: 0,
                width: 8,
                height: 4,
                rgba: vec![v; 8 * 4 * 4],
            },
            Shot {
                x: 8,
                y: 0,
                width: 8,
                height: 4,
                rgba: vec![v; 8 * 4 * 4],
            },
        ]
    }

    fn black_at(img: &image::RgbaImage, x: u32, y: u32) -> bool {
        img.get_pixel(x, y).0 == [0, 0, 0, 255]
    }

    #[test]
    fn a_secret_on_the_second_screen_is_blanked_though_it_never_had_focus() {
        // The case a foreground test cannot see: the password manager is open
        // on the right-hand monitor while the browser has focus on the left.
        let secret = Secret {
            left: 10,
            top: 1,
            right: 14,
            bottom: 3,
        };
        let img = composite(desk(200), &[secret]).unwrap();

        assert!(black_at(&img, 10, 1), "the secret's own pixels survived");
        assert!(black_at(&img, 13, 2), "and its far corner");
        assert!(!black_at(&img, 9, 1), "the pixel left of it was eaten");
        assert!(!black_at(&img, 14, 1), "the pixel right of it was eaten");
        assert!(!black_at(&img, 2, 2), "the other screen was blanked too");
    }

    #[test]
    fn a_screen_left_of_the_origin_still_blanks_the_right_pixels() {
        // Windows puts a second monitor at a NEGATIVE x when it sits to the
        // left of the primary. The composite packs left to right from zero, so
        // the two spaces disagree, and a rectangle that skips the translation
        // lands on the wrong screen while still looking blanked.
        let shots = vec![
            Shot {
                x: -8,
                y: 0,
                width: 8,
                height: 4,
                rgba: vec![120; 8 * 4 * 4],
            },
            Shot {
                x: 0,
                y: 0,
                width: 8,
                height: 4,
                rgba: vec![120; 8 * 4 * 4],
            },
        ];
        // A window at desktop x=-6 is on the LEFT screen, composite x=2.
        let img = composite(
            shots,
            &[Secret {
                left: -6,
                top: 0,
                right: -4,
                bottom: 2,
            }],
        )
        .unwrap();

        assert!(black_at(&img, 2, 0), "the left screen's rectangle");
        assert!(black_at(&img, 3, 1), "and the rest of it");
        assert!(!black_at(&img, 10, 0), "the right screen was hit instead");
        assert!(!black_at(&img, 6, 0), "the rectangle slid along the screen");
    }

    #[test]
    fn a_secret_spanning_both_screens_is_blanked_on_each() {
        let img = composite(
            desk(90),
            &[Secret {
                left: 6,
                top: 0,
                right: 10,
                bottom: 4,
            }],
        )
        .unwrap();

        assert!(black_at(&img, 7, 2), "the half on the first screen");
        assert!(black_at(&img, 9, 2), "the half on the second");
        assert!(!black_at(&img, 5, 2), "it grew leftward");
        assert!(!black_at(&img, 11, 2), "it grew rightward");
    }

    #[test]
    fn a_rectangle_reaching_past_the_desk_is_clipped_rather_than_panicking() {
        // A minimised window reports coordinates far off any screen, and a
        // maximised one reports slightly past the edge. Neither may index out
        // of the canvas.
        let far = Secret {
            left: -32000,
            top: -32000,
            right: -31000,
            bottom: -31000,
        };
        let over = Secret {
            left: 12,
            top: 2,
            right: 9999,
            bottom: 9999,
        };
        let img = composite(desk(140), &[far, over]).unwrap();

        assert!(black_at(&img, 15, 3), "the clipped rectangle still blanked");
        assert!(
            !black_at(&img, 0, 0),
            "the off-screen one blanked something"
        );
    }

    #[test]
    fn the_hash_is_taken_after_blanking_so_a_hidden_change_is_not_a_new_frame() {
        let mut rec = Recorder::new(Fake::new(), scratch(), 0, vec!["nordpass.exe".into()]);
        *rec.source.secrets.borrow_mut() = vec![Secret {
            left: 0,
            top: 0,
            right: 16,
            bottom: 4,
        }];

        assert!(matches!(
            rec.tick(("2026-08-21", "0900"), true).unwrap(),
            Tick::Kept { .. }
        ));

        // The screen changed, but only underneath the blanked rectangle.
        *rec.source.pixel.borrow_mut() = 250;
        assert_eq!(
            rec.tick(("2026-08-21", "0901"), true).unwrap(),
            Tick::Unchanged
        );
    }
}
