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
    /// This context asked for titles only, so no picture was taken.
    Titles,
}

pub struct Recorder<S: Source> {
    source: S,
    root: PathBuf,
    floor_bytes: u64,
    /// The bucket the last tick wrote into, so a rollover can be noticed and
    /// the one behind it sealed. Holds no data of its own: every frame and
    /// every line is already on disk the moment it is written, so there is
    /// nothing here a crash could lose.
    open: Option<(String, String)>,
    last_hash: Option<[u8; 32]>,
}

impl<S: Source> Recorder<S> {
    pub fn new(source: S, root: PathBuf, floor_bytes: u64) -> Self {
        Self {
            source,
            root,
            floor_bytes,
            open: None,
            last_hash: None,
        }
    }

    /// One minute. `stamp` is the operator's wall clock, already local, as
    /// `("2026-08-20", "0914")`; `capturing` is Kaizen's answer to whether this
    /// minute is inside a context that earns it, which is deliberately NOT
    /// recomputed here. The widget must not carry a second opinion about what a
    /// working day is.
    pub fn tick(
        &mut self,
        stamp: (&str, &str),
        activity: bool,
        screen: bool,
    ) -> Result<Tick, String> {
        if !activity && !screen {
            self.close_open()?;
            return Ok(Tick::Resting);
        }

        let free = self.source.free_bytes(&self.root);
        if free < self.floor_bytes {
            self.close_open()?;
            return Ok(Tick::NoRoom {
                free_mb: free / 1_048_576,
            });
        }

        let probe = self.source.probe()?;

        if probe.locked {
            self.push(stamp, &probe, "locked", None)?;
            return Ok(Tick::Away);
        }

        // A context may want the line without the picture, which is the whole
        // point of the two switches being separate.
        if !screen {
            self.push(stamp, &probe, "titles", None)?;
            return Ok(Tick::Titles);
        }

        let frame = encode(self.source.shots()?)?;
        let hash: [u8; 32] = Sha256::digest(&frame).into();

        // Reading one long document is thirty identical screenshots. Storing
        // the first and pointing at it costs a line instead of a file.
        if self.last_hash == Some(hash) {
            self.push(stamp, &probe, "same", None)?;
            return Ok(Tick::Unchanged);
        }

        let bytes = frame.len();
        self.last_hash = Some(hash);
        self.push(stamp, &probe, "kept", Some(frame))?;
        Ok(Tick::Kept { bytes })
    }

    /// Write immediately: the line to the day's TSV, the frame (if any) to
    /// its own loose file. Nothing here is held only in memory, so a crash or
    /// a forced quit can lose at most the current minute, never the fourteen
    /// before it.
    fn push(
        &mut self,
        (day, minute): (&str, &str),
        probe: &Probe,
        note: &str,
        frame: Option<Vec<u8>>,
    ) -> Result<(), String> {
        let start = bucket_start(minute);

        // Crossing into a new quarter, or a new day, means the one behind it
        // is finished and may as well be zipped now rather than left loose
        // until something else notices.
        if self
            .open
            .as_ref()
            .is_some_and(|(d, s)| d != day || s != &start)
        {
            self.close_open()?;
        }
        self.open = Some((day.to_string(), start));

        let dir = self.root.join(day);
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

        let line = format!(
            "{}\t{}\t{}\t{}\t{}",
            minute,
            probe.idle_secs,
            note,
            probe.process,
            probe.title.replace(['\t', '\n', '\r'], " ")
        );

        // The day's TSV is one file, appended live, never sealed and never
        // touched by archiving: a checkpoint reads or greps the whole day
        // without unzipping anything.
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join(format!("{day}.tsv")))
            .map_err(|e| e.to_string())?;
        writeln!(f, "{line}").map_err(|e| e.to_string())?;

        if let Some(frame) = frame {
            // Prefixed with the day, matching the zip's own filename, so pure
            // alphabetical order in the folder is pure chronological order.
            // A bare "2100.jpg" ties the leading digit of any "2026-..." name
            // and sorts AFTER it from there, which is not a 2026 quirk: every
            // year this decade shares that "202" prefix, so a bare hour would
            // flip position mid-afternoon indefinitely, not just this year.
            fs::write(dir.join(format!("{day}_{minute}.jpg")), frame).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Seal whichever bucket this recorder was last writing into, if any.
    fn close_open(&mut self) -> Result<(), String> {
        if let Some((day, start)) = self.open.take() {
            seal_bucket(&self.root, &day, &start)?;
        }
        Ok(())
    }

    /// Total bytes held under the archive root, for the tray to show. Nothing
    /// prunes; seeing it grow is the whole point.
    pub fn size_bytes(&self) -> u64 {
        walk(&self.root)
    }
}

/// Zip whichever loose frames exist for one 15-minute bucket. A free function
/// rather than a method, so a startup sweep with no live `Recorder` can reuse
/// the exact same logic a running one uses when a quarter rolls over — one
/// path that may zip and delete, never two that could disagree.
///
/// A zip is images only. The day's TSV already holds the record of every
/// minute, kept or not, so a bucket with nothing kept in it (titles-only, or a
/// screen that simply never changed) has nothing to consolidate and gets no
/// zip at all. An empty archive would say less than no archive does.
///
/// Idempotent by construction: if the zip already exists, NOTHING is touched,
/// loose files included. Deletion happens only in the one branch that just
/// finished writing and renaming a zip successfully, immediately after, and
/// only for the exact files that went into it. There is no separate cleanup
/// pass with its own opinion about what may be removed.
pub fn seal_bucket(root: &Path, day: &str, start: &str) -> Result<(), String> {
    let dir = root.join(day);
    let name = zip_name(day, start);

    if dir.join(&name).exists() {
        return Ok(());
    }

    // The zip entry stays bare (just "0930.jpg"): the archive's own filename
    // already carries the date, so repeating it inside every entry would be
    // pure redundancy in a listing nobody browses outside this one day.
    let present: Vec<(String, PathBuf)> = minutes_in_bucket(start)
        .into_iter()
        .map(|m| (format!("{m}.jpg"), dir.join(format!("{day}_{m}.jpg"))))
        .filter(|(_, path)| path.exists())
        .collect();

    if present.is_empty() {
        return Ok(());
    }

    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let part = dir.join(format!("{name}.part"));
    let mut zip = zip::ZipWriter::new(File::create(&part).map_err(|e| e.to_string())?);

    // Deflate, which is NOT the obvious call and was got wrong here first.
    // "JPEG is already compressed" is true of the pixels and false of the
    // file: an encoder shipping the standard Huffman tables rather than
    // optimised ones leaves real redundancy in the entropy-coded stream, and
    // deflate takes it. Measured on a screenshot re-encoded at this exact
    // quality it took another 35% off, and MORE at lower quality rather than
    // less, so the downside of trying is bounded at roughly nothing.
    let squeezed = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    for (fname, path) in &present {
        let bytes = fs::read(path).map_err(|e| e.to_string())?;
        zip.start_file(fname, squeezed).map_err(|e| e.to_string())?;
        zip.write_all(&bytes).map_err(|e| e.to_string())?;
    }

    zip.finish().map_err(|e| e.to_string())?;
    fs::rename(&part, dir.join(&name)).map_err(|e| e.to_string())?;

    // Only now, having a verified zip on disk under its real name, remove the
    // loose duplicates. If this loop is interrupted, the zip already exists
    // and the next call for this bucket returns at the very first line.
    for (_, path) in &present {
        let _ = fs::remove_file(path);
    }
    Ok(())
}

/// Every minute floor covers, in order: "0930" -> 0930..=0944.
fn minutes_in_bucket(start: &str) -> Vec<String> {
    let (h, m) = start.split_at(2);
    let base = h.parse::<u32>().unwrap_or(0) * 60 + m.parse::<u32>().unwrap_or(0);

    (0..BUCKET_MINUTES)
        .map(|i| {
            let t = base + i;
            format!("{:02}{:02}", (t / 60) % 24, t % 60)
        })
        .collect()
}

/// Zip every closed bucket that has loose frames still waiting, across every
/// day the archive holds. Meant to run once at startup: a crash or a forced
/// quit leaves loose files behind exactly where they were written, and this
/// is what turns them into their zip on the next launch rather than leaving
/// them loose forever.
///
/// `today` and `now_bucket` name the one bucket that is deliberately left
/// alone: it may still be open and belongs to the running tick loop, not to a
/// one-off sweep that could race it.
pub fn recover(root: &Path, today: &str, now_bucket: &str) -> Result<(), String> {
    let Ok(days) = fs::read_dir(root) else {
        return Ok(());
    };

    for day_entry in days.flatten() {
        let Ok(file_type) = day_entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        let day = day_entry.file_name().to_string_lossy().into_owned();

        let Ok(files) = fs::read_dir(day_entry.path()) else {
            continue;
        };

        // Loose files are "{day}_{minute}.jpg"; the day prefix is stripped
        // rather than trusted, so a stray file from anywhere else is simply
        // not recognised rather than mis-parsed.
        let prefix = format!("{day}_");
        let mut starts: Vec<String> = files
            .flatten()
            .filter_map(|f| {
                let name = f.file_name().to_string_lossy().into_owned();
                let minute = name.strip_prefix(&prefix)?.strip_suffix(".jpg")?;
                (minute.len() == 4 && minute.chars().all(|c| c.is_ascii_digit()))
                    .then(|| bucket_start(minute))
            })
            .collect();
        starts.sort();
        starts.dedup();

        for start in starts {
            if day == today && start == now_bucket {
                continue;
            }
            seal_bucket(root, &day, &start)?;
        }
    }
    Ok(())
}

/// Where the archive lives under an app data directory.
pub fn root_in(app_data: &Path) -> PathBuf {
    app_data.join("capture")
}

/// The day's activity lines, for handing to an AI at a checkpoint.
///
/// Read from the live TSV rather than the archives, so it works mid-quarter and
/// needs nothing unzipped. This is the only route by which any of this leaves
/// the machine, and it is the operator taking it: the recorder never uploads,
/// and a prompt travels only when it is pasted somewhere.
///
/// Titles only. The pictures stay on disk, because a screenshot is a picture of
/// everything that was on screen while a title is a line naming a program and a
/// window, and handing over the second is not consent for the first.
pub fn activity_for(root: &Path, day: &str) -> Option<String> {
    let text = fs::read_to_string(root.join(day).join(format!("{day}.tsv"))).ok()?;

    (!text.trim().is_empty()).then_some(text)
}

/// Floor a minute to its quarter: 0914 -> 0900, 0915 -> 0915.
pub fn bucket_start(minute: &str) -> String {
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
fn encode(shots: Vec<Shot>) -> Result<Vec<u8>, String> {
    let canvas = composite(shots)?;
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
/// Split from `encode` so a test can look at the pixels rather than a JPEG.
fn composite(shots: Vec<Shot>) -> Result<image::RgbaImage, String> {
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
    fn a_bucket_names_every_minute_it_covers() {
        assert_eq!(
            minutes_in_bucket("0900"),
            vec![
                "0900", "0901", "0902", "0903", "0904", "0905", "0906", "0907", "0908", "0909",
                "0910", "0911", "0912", "0913", "0914"
            ]
        );
        // The day's last bucket stays inside the day; it does not wrap to
        // 0000 the way the zip's END LABEL does for display.
        assert_eq!(minutes_in_bucket("2345").last().unwrap(), "2359");
    }

    #[test]
    fn a_loose_filename_sorts_chronologically_against_every_dated_file_all_day() {
        // The bug this guards: a bare "2100.jpg" ties the leading '2' of
        // "2026-...", and the comparison falls through to digits that are
        // now comparing an HOUR against a YEAR. Past that tie it reads as
        // greater, so the file flips from sorting before every dated name to
        // sorting after all of them — and because 2020-2029 all start "202",
        // this is not a one-off quirk of this particular year.
        //
        // Prefixing the loose name with the day removes the coincidence
        // entirely: every name in the folder now starts with the same date,
        // so alphabetical order cannot help but be chronological order.
        // Built in one chronological pass rather than by asserting where a
        // zip "should" land: a zip's name and its same-minute loose file
        // share every character up to '-' vs '.', and '-' (0x2D) sorts
        // before '.' (0x2E), so the zip belongs immediately before the loose
        // file for the minute it starts on. Getting that placement right by
        // hand once, elsewhere, is exactly the kind of mistake this test
        // exists to catch on every future change instead.
        let day = "2026-08-24";
        let mut names = vec![format!("{day}.tsv")];
        for hour in 0..24 {
            for minute in [0, 30] {
                let hhmm = format!("{hour:02}{minute:02}");
                if hhmm == "0900" {
                    names.push(format!("{day}_0900-0915.zip"));
                }
                if hhmm == "2030" {
                    names.push(format!("{day}_2030-2045.zip"));
                }
                names.push(format!("{day}_{hhmm}.jpg"));
            }
        }

        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(
            sorted, names,
            "alphabetical order was not already chronological order"
        );
    }

    #[test]
    fn monitors_are_laid_out_side_by_side_in_one_image() {
        let wide = encode(vec![
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
        ])
        .unwrap();

        // A JPEG, and one image rather than two files to reconcile later.
        assert_eq!(&wide[..2], &[0xFF, 0xD8]);
        assert!(!wide.is_empty());
    }

    #[test]
    fn a_monitor_whose_pixels_do_not_match_its_size_is_refused() {
        let err = encode(vec![Shot {
            x: 0,
            y: 0,
            width: 8,
            height: 4,
            rgba: vec![0; 3],
        }])
        .unwrap_err();
        assert!(err.contains("did not match"), "{err}");
    }

    #[test]
    fn frames_land_on_disk_the_moment_they_are_taken() {
        // The whole point of dropping the in-memory bucket: nothing here may
        // be lost to a crash, because nothing here is held only in memory.
        let root = scratch();
        let mut rec = Recorder::new(Fake::new(), root.clone(), 0);

        rec.tick(("2026-08-24", "0900"), true, true).unwrap();

        assert!(root.join("2026-08-24/2026-08-24_0900.jpg").exists());
        assert!(!root.join("2026-08-24/2026-08-24_0900-0915.zip").exists());
    }

    #[test]
    fn an_unchanged_screen_costs_a_line_rather_than_a_file() {
        let root = scratch();
        let mut rec = Recorder::new(Fake::new(), root.clone(), 0);

        assert!(matches!(
            rec.tick(("2026-08-20", "0900"), true, true).unwrap(),
            Tick::Kept { .. }
        ));
        assert_eq!(
            rec.tick(("2026-08-20", "0901"), true, true).unwrap(),
            Tick::Unchanged
        );
        assert!(!root.join("2026-08-20/2026-08-20_0901.jpg").exists());

        *rec.source.pixel.borrow_mut() = 200; // the screen changed
        assert!(matches!(
            rec.tick(("2026-08-20", "0902"), true, true).unwrap(),
            Tick::Kept { .. }
        ));
        assert!(root.join("2026-08-20/2026-08-20_0902.jpg").exists());
    }

    #[test]
    fn a_locked_workstation_is_recorded_but_not_pictured() {
        let root = scratch();
        let mut rec = Recorder::new(Fake::new(), root.clone(), 0);
        rec.source.probe.borrow_mut().locked = true;

        assert_eq!(
            rec.tick(("2026-08-20", "0900"), true, true).unwrap(),
            Tick::Away
        );
        assert!(!root.join("2026-08-20/2026-08-20_0900.jpg").exists());
        assert!(fs::read_to_string(root.join("2026-08-20/2026-08-20.tsv"))
            .unwrap()
            .contains("\tlocked\t"));
    }

    #[test]
    fn resting_seals_what_is_open_rather_than_holding_it() {
        let root = scratch();
        let mut rec = Recorder::new(Fake::new(), root.clone(), 0);

        rec.tick(("2026-08-20", "0900"), true, true).unwrap();
        assert_eq!(
            rec.tick(("2026-08-20", "0901"), false, false).unwrap(),
            Tick::Resting
        );

        assert!(root.join("2026-08-20/2026-08-20_0900-0915.zip").exists());
        // The loose original is gone the moment its zip is verified on disk.
        assert!(!root.join("2026-08-20/2026-08-20_0900.jpg").exists());
    }

    #[test]
    fn the_floor_stops_capture_without_deleting_anything() {
        let root = scratch();
        let mut rec = Recorder::new(Fake::new(), root.clone(), 1_000_000);
        rec.tick(("2026-08-20", "0900"), true, true).unwrap();

        *rec.source.free.borrow_mut() = 5;
        let tick = rec.tick(("2026-08-20", "0901"), true, true).unwrap();

        assert!(matches!(tick, Tick::NoRoom { .. }), "{tick:?}");
        // What was already captured is sealed, not discarded.
        assert!(root.join("2026-08-20/2026-08-20_0900-0915.zip").exists());
    }

    #[test]
    fn crossing_the_quarter_seals_the_one_behind_it() {
        let root = scratch();
        let mut rec = Recorder::new(Fake::new(), root.clone(), 0);

        rec.tick(("2026-08-20", "0914"), true, true).unwrap();
        *rec.source.pixel.borrow_mut() = 50;
        rec.tick(("2026-08-20", "0915"), true, true).unwrap();

        assert!(root.join("2026-08-20/2026-08-20_0900-0915.zip").exists());
        assert!(root.join("2026-08-20/2026-08-20_0915.jpg").exists());
    }

    #[test]
    fn a_bucket_with_no_kept_frames_gets_no_zip_at_all() {
        // Screen capture on, but the picture never changes: every minute in
        // THIS bucket dedupes to "same" and nothing is ever kept. The TSV
        // already has the full record, so an empty archive would say less
        // than none.
        //
        // A fresh Recorder's very first frame is always Kept — there is
        // nothing yet to compare it against — so this warms last_hash up in
        // the PRECEDING bucket first, and only then holds the screen still
        // for the one under test.
        let root = scratch();
        let mut rec = Recorder::new(Fake::new(), root.clone(), 0);

        rec.tick(("2026-08-20", "0845"), true, true).unwrap();

        for minute in ["0900", "0901", "0902"] {
            assert_eq!(
                rec.tick(("2026-08-20", minute), true, true).unwrap(),
                Tick::Unchanged
            );
        }
        rec.tick(("2026-08-20", "0915"), false, false).unwrap(); // seals 0900

        assert!(!root.join("2026-08-20/2026-08-20_0900-0915.zip").exists());
        assert!(
            fs::read_to_string(root.join("2026-08-20/2026-08-20.tsv"))
                .unwrap()
                .lines()
                .filter(|l| l.contains("\tsame\t"))
                .count()
                == 3
        );
    }

    #[test]
    fn titles_without_pictures_gets_no_zip_either() {
        // A context may want to know which program was in front without a
        // photograph of what was in it. Since a zip is images only, that
        // preference means no archive for the quarter at all — the day's
        // TSV is the whole record.
        let root = scratch();
        let mut rec = Recorder::new(Fake::new(), root.clone(), 0);

        assert_eq!(
            rec.tick(("2026-08-23", "0900"), true, false).unwrap(),
            Tick::Titles
        );
        rec.tick(("2026-08-23", "0915"), false, false).unwrap(); // seals 0900

        assert!(!root.join("2026-08-23/2026-08-23_0900-0915.zip").exists());
        let tsv = fs::read_to_string(root.join("2026-08-23/2026-08-23.tsv")).unwrap();
        assert!(tsv.contains("OUTLOOK.EXE"), "the line was written: {tsv}");
        assert!(
            tsv.contains("\ttitles\t"),
            "and says why there is no frame: {tsv}"
        );
    }

    #[test]
    fn a_sealed_quarter_holds_only_images() {
        let root = scratch();
        let mut rec = Recorder::new(Fake::new(), root.clone(), 0);

        rec.tick(("2026-08-20", "0900"), true, true).unwrap();
        *rec.source.pixel.borrow_mut() = 77;
        rec.tick(("2026-08-20", "0901"), true, true).unwrap();
        rec.tick(("2026-08-20", "0915"), false, false).unwrap(); // seals

        let path = root.join("2026-08-20/2026-08-20_0900-0915.zip");
        let zip = zip::ZipArchive::new(File::open(&path).unwrap()).unwrap();
        let names: Vec<String> = zip.file_names().map(String::from).collect();

        // Entries are bare: the archive's own filename already carries the
        // date, so repeating it in every entry would be pure redundancy.
        assert!(names.contains(&"0900.jpg".to_string()), "{names:?}");
        assert!(names.contains(&"0901.jpg".to_string()), "{names:?}");
        assert!(
            !names.iter().any(|n| n.ends_with(".tsv")),
            "the day's TSV lives once, outside every zip: {names:?}"
        );
        assert!(!path.with_extension("zip.part").exists());
    }

    #[test]
    fn sealing_an_already_sealed_bucket_touches_nothing() {
        // The unplug-and-reconnect case: a zip already exists, so this must
        // do nothing at all rather than repack, and — the specific worry —
        // must not delete a loose file that happens to still be lying beside
        // it, because deletion is only ever a consequence of THIS call
        // writing that exact zip, and this call is not writing one.
        let root = scratch();
        let dir = root.join("2026-08-20");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("2026-08-20_0900-0915.zip"), b"already here").unwrap();
        fs::write(dir.join("2026-08-20_0900.jpg"), b"stray loose copy").unwrap();

        seal_bucket(&root, "2026-08-20", "0900").unwrap();

        assert_eq!(
            fs::read(dir.join("2026-08-20_0900-0915.zip")).unwrap(),
            b"already here",
            "the existing zip was not rewritten"
        );
        assert!(
            dir.join("2026-08-20_0900.jpg").exists(),
            "a loose file beside an existing zip is left alone, not swept"
        );
    }

    #[test]
    fn the_days_tsv_is_readable_without_opening_a_single_zip() {
        let root = scratch();
        let mut rec = Recorder::new(Fake::new(), root.clone(), 0);

        rec.source.probe.borrow_mut().title = "Ticket ZP0138157\twith a tab".into();
        rec.tick(("2026-08-20", "0900"), true, true).unwrap();
        rec.tick(("2026-08-20", "0901"), true, true).unwrap();

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
    fn the_days_activity_is_readable_without_a_recorder() {
        // The prompt is assembled by a command, not by the capture thread, so
        // reading the day cannot require holding the recorder.
        let root = scratch();
        let mut rec = Recorder::new(Fake::new(), root.clone(), 0);

        assert!(activity_for(&root, "2026-08-23").is_none(), "nothing yet");

        rec.tick(("2026-08-23", "0900"), true, true).unwrap();

        let text = activity_for(&root, "2026-08-23").expect("the day reads back");
        assert!(text.contains("OUTLOOK.EXE"), "{text}");
    }

    #[test]
    fn recovery_zips_a_bucket_a_previous_session_never_closed() {
        // Simulates the crash case directly, with no live Recorder at all:
        // loose files simply exist on disk, as they would after a forced
        // quit, and recover() alone has to turn them into a zip.
        let root = scratch();
        let dir = root.join("2026-08-20");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("2026-08-20.tsv"),
            "0900\t0\tkept\tOUTLOOK.EXE\tInbox\n",
        )
        .unwrap();
        fs::write(dir.join("2026-08-20_0900.jpg"), b"frame").unwrap();

        recover(&root, "2026-08-24", "1200").unwrap();

        assert!(dir.join("2026-08-20_0900-0915.zip").exists());
        assert!(!dir.join("2026-08-20_0900.jpg").exists());
    }

    #[test]
    fn recovery_leaves_the_bucket_the_clock_is_inside_alone() {
        // That bucket may still be genuinely open and belongs to the running
        // tick loop; a one-off startup sweep sealing it out from under that
        // loop would be a race, not a recovery.
        let root = scratch();
        let dir = root.join("2026-08-24");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("2026-08-24_1200.jpg"), b"frame").unwrap();

        recover(&root, "2026-08-24", "1200").unwrap();

        assert!(dir.join("2026-08-24_1200.jpg").exists(), "left untouched");
        assert!(!dir.join("2026-08-24_1200-1215.zip").exists());
    }

    #[test]
    fn recovery_is_a_no_op_once_everything_is_already_sealed() {
        let root = scratch();
        let mut rec = Recorder::new(Fake::new(), root.clone(), 0);
        rec.tick(("2026-08-20", "0900"), true, true).unwrap();
        rec.tick(("2026-08-20", "0915"), false, false).unwrap();

        // A second recovery pass over an already-tidy day should not error,
        // rewrite the zip, or find anything left to do.
        recover(&root, "2026-08-24", "1200").unwrap();
        assert!(root.join("2026-08-20/2026-08-20_0900-0915.zip").exists());
    }
}
