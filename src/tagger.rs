//! Everything that talks to ExifTool or interprets its output. No UI code lives here.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const PHOTO_EXTS: &[&str] = &[
    "dng", "jpg", "jpeg", "tif", "tiff", "heic", "cr2", "cr3", "arw", "nef", "orf", "raf", "rw2",
];

fn has_ext(path: &Path, exts: &[&str]) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| exts.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

pub fn is_photo(path: &Path) -> bool {
    has_ext(path, PHOTO_EXTS)
}

pub fn is_gpx(path: &Path) -> bool {
    has_ext(path, &["gpx"])
}

/// Replaces folders with the files inside them (recursively); plain files pass through.
pub fn expand_dirs(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for p in paths {
        if p.is_dir() {
            if let Ok(rd) = std::fs::read_dir(&p) {
                let mut children: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
                children.sort();
                out.extend(expand_dirs(children));
            }
        } else {
            out.push(p);
        }
    }
    out
}

pub fn find_exiftool() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = vec![
        "/opt/homebrew/bin/exiftool".into(), // Apple Silicon Homebrew
        "/usr/local/bin/exiftool".into(),    // Intel Homebrew / installer
        "/usr/bin/exiftool".into(),          // Linux
        r"C:\Windows\exiftool.exe".into(),
        r"C:\Program Files\exiftool\exiftool.exe".into(),
    ];
    candidates.push(PathBuf::from(if cfg!(windows) { "exiftool.exe" } else { "exiftool" }));
    candidates.into_iter().find(|c| {
        Command::new(c)
            .arg("-ver")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    })
}

#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    /// Camera clock offset from UTC, e.g. "-03:00".
    pub tz: String,
    /// Seconds added to every photo time (positive if the camera clock runs behind).
    pub drift_secs: i64,
    pub keep_backup: bool,
    pub replace_existing_gps: bool,
}

pub fn build_args(gpx: &[PathBuf], opts: &Options, photo: &Path) -> Vec<OsString> {
    let mut args: Vec<OsString> = Vec::new();
    for g in gpx {
        args.push("-geotag".into());
        args.push(g.into());
    }
    args.push(format!("-geotime<${{DateTimeOriginal}}{}", opts.tz).into());
    if opts.drift_secs != 0 {
        args.push(format!("-geosync={:+}", opts.drift_secs).into());
    }
    if !opts.replace_existing_gps {
        args.push("-if".into());
        args.push("not $GPSLatitude".into());
    }
    if !opts.keep_backup {
        args.push("-overwrite_original".into());
    }
    args.push(photo.into());
    args
}

#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Tagged,
    AlreadyHasGps,
    NoMatch(String),
    Failed(String),
}

pub fn classify(stdout: &str, stderr: &str, success: bool) -> Outcome {
    let all = format!("{stdout}\n{stderr}");
    if all.contains("1 image files updated") {
        Outcome::Tagged
    } else if all.contains("failed condition") {
        Outcome::AlreadyHasGps
    } else if !success {
        Outcome::Failed(first_line(&all))
    } else {
        Outcome::NoMatch(first_line(stderr))
    }
}

fn first_line(s: &str) -> String {
    s.lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.contains("image files"))
        .unwrap_or("")
        .to_string()
}

/// Tags one photo.
pub fn tag_one(exiftool: &Path, gpx: &[PathBuf], opts: &Options, photo: &Path) -> Outcome {
    match Command::new(exiftool).args(build_args(gpx, opts, photo)).output() {
        Ok(o) => classify(
            &String::from_utf8_lossy(&o.stdout),
            &String::from_utf8_lossy(&o.stderr),
            o.status.success(),
        ),
        Err(e) => Outcome::Failed(format!("Could not run ExifTool: {e}")),
    }
}

// ---------------------------------------------------------------- scanning

#[derive(Clone, Debug, Default)]
pub struct PhotoInfo {
    pub path: PathBuf,
    /// Capture time as seconds since epoch, treating the camera's local clock as UTC.
    pub local_time: Option<i64>,
    /// `OffsetTimeOriginal`, e.g. "-03:00".
    pub offset: Option<String>,
    pub has_gps: bool,
}

/// Reads capture time, time zone offset and existing GPS for each photo in one ExifTool call.
pub fn scan(exiftool: &Path, paths: &[PathBuf]) -> Vec<PhotoInfo> {
    let out = Command::new(exiftool)
        .args(["-T", "-f", "-DateTimeOriginal", "-OffsetTimeOriginal", "-GPSLatitude"])
        .args(paths)
        .output();
    match out {
        Ok(o) => parse_scan_output(paths, &String::from_utf8_lossy(&o.stdout)),
        Err(_) => paths.iter().map(|p| PhotoInfo { path: p.clone(), ..Default::default() }).collect(),
    }
}

pub fn parse_scan_output(paths: &[PathBuf], out: &str) -> Vec<PhotoInfo> {
    let mut lines = out.lines();
    paths
        .iter()
        .map(|path| {
            let line = lines.next().unwrap_or("");
            let mut cols = line.split('\t').map(str::trim);
            let dt = cols.next().unwrap_or("-");
            let off = cols.next().unwrap_or("-");
            let lat = cols.next().unwrap_or("-");
            PhotoInfo {
                path: path.clone(),
                local_time: parse_exif_datetime(dt),
                offset: (off != "-" && parse_offset(off).is_some()).then(|| off.to_string()),
                has_gps: lat != "-" && !lat.is_empty(),
            }
        })
        .collect()
}

// ---------------------------------------------------------------- time helpers

/// Days since 1970-01-01 for a civil date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn to_epoch(y: i64, mo: i64, d: i64, h: i64, mi: i64, s: i64) -> Option<i64> {
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 24 || mi > 59 || s > 60 {
        return None;
    }
    Some(days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 + s)
}

/// "2024:05:01 12:30:00" (anything after the seconds is ignored).
pub fn parse_exif_datetime(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19 {
        return None;
    }
    let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    to_epoch(n(0..4)?, n(5..7)?, n(8..10)?, n(11..13)?, n(14..16)?, n(17..19)?)
}

/// "-03:00" -> -10800.
pub fn parse_offset(s: &str) -> Option<i64> {
    let sign = match s.as_bytes().first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let (h, m) = s.get(1..)?.split_once(':')?;
    Some(sign * (h.parse::<i64>().ok()? * 3600 + m.parse::<i64>().ok()? * 60))
}

/// "2024-05-01T12:30:00Z" or with a "+hh:mm" offset or fractional seconds.
pub fn parse_iso_datetime(s: &str) -> Option<i64> {
    let s = s.trim();
    let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let base = to_epoch(n(0..4)?, n(5..7)?, n(8..10)?, n(11..13)?, n(14..16)?, n(17..19)?)?;
    let mut rest = &s[19..];
    if let Some(frac) = rest.strip_prefix('.') {
        rest = frac.trim_start_matches(|c: char| c.is_ascii_digit());
    }
    let off = if rest.is_empty() || rest == "Z" { 0 } else { parse_offset(rest)? };
    Some(base - off)
}

/// First and last `<time>` value in a GPX file (UTC epoch seconds).
pub fn gpx_range(path: &Path) -> Option<(i64, i64)> {
    let text = std::fs::read_to_string(path).ok()?;
    gpx_range_from_text(&text)
}

pub fn gpx_range_from_text(text: &str) -> Option<(i64, i64)> {
    let mut range: Option<(i64, i64)> = None;
    let mut rest = text;
    while let Some(i) = rest.find("<time>") {
        rest = &rest[i + 6..];
        let end = rest.find("</time>")?;
        if let Some(t) = parse_iso_datetime(&rest[..end]) {
            range = Some(match range {
                Some((lo, hi)) => (lo.min(t), hi.max(t)),
                None => (t, t),
            });
        }
        rest = &rest[end..];
    }
    range
}

pub fn format_epoch(t: i64) -> String {
    let days = t.div_euclid(86400);
    let secs = t.rem_euclid(86400);
    // civil_from_days
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}", secs / 3600, (secs % 3600) / 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exif_and_iso_times_agree() {
        let a = parse_exif_datetime("2024:05:01 12:30:00").unwrap();
        let b = parse_iso_datetime("2024-05-01T12:30:00Z").unwrap();
        assert_eq!(a, b);
        assert_eq!(parse_iso_datetime("2024-05-01T09:30:00.250-03:00").unwrap(), b);
        assert_eq!(parse_exif_datetime("-"), None);
    }

    #[test]
    fn epoch_round_trip() {
        let t = parse_exif_datetime("2024:02:29 23:59:00").unwrap();
        assert_eq!(format_epoch(t), "2024-02-29 23:59");
        assert_eq!(parse_exif_datetime("1970:01:01 00:00:00"), Some(0));
    }

    #[test]
    fn offsets() {
        assert_eq!(parse_offset("-03:00"), Some(-10800));
        assert_eq!(parse_offset("+05:30"), Some(19800));
        assert_eq!(parse_offset("UTC"), None);
    }

    #[test]
    fn gpx_range_finds_min_and_max() {
        let gpx = "<trkpt><time>2024-05-01T10:00:00Z</time></trkpt>\
                   <trkpt><time>2024-05-01T12:00:00Z</time></trkpt>\
                   <trkpt><time>2024-05-01T11:00:00Z</time></trkpt>";
        let (lo, hi) = gpx_range_from_text(gpx).unwrap();
        assert_eq!(hi - lo, 7200);
    }

    #[test]
    fn args_respect_options() {
        let opts = Options {
            tz: "-03:00".into(),
            drift_secs: 30,
            keep_backup: true,
            replace_existing_gps: false,
        };
        let args: Vec<String> = build_args(&["a.gpx".into()], &opts, Path::new("p.dng"))
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(args.contains(&"-geosync=+30".to_string()));
        assert!(args.contains(&"not $GPSLatitude".to_string()));
        assert!(!args.contains(&"-overwrite_original".to_string()));
        assert!(args.contains(&"-geotime<${DateTimeOriginal}-03:00".to_string()));
    }

    #[test]
    fn classifies_exiftool_output() {
        assert_eq!(classify("    1 image files updated", "", true), Outcome::Tagged);
        assert_eq!(
            classify("    0 image files updated\n    1 files failed condition", "", true),
            Outcome::AlreadyHasGps
        );
        assert!(matches!(classify("0 image files updated", "Warning: x", true), Outcome::NoMatch(_)));
        assert!(matches!(classify("", "Error: File not found", false), Outcome::Failed(_)));
    }

    #[test]
    fn parses_scan_rows() {
        let paths = vec![PathBuf::from("a.dng"), PathBuf::from("b.dng")];
        let out = "2024:05:01 12:30:00\t-03:00\t-\n-\t-\t12 deg 1' 2.00\" S\n";
        let info = parse_scan_output(&paths, out);
        assert_eq!(info[0].offset.as_deref(), Some("-03:00"));
        assert!(!info[0].has_gps);
        assert!(info[1].local_time.is_none() && info[1].has_gps);
    }
}
