//! Expert sounds saved for restore: one tab-separated line each in `saved.tsv`.
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct Saved {
    pub name: String,
    /// `{brand} {name}` of the mixer it was saved on.
    pub mixer: String,
    /// Channel group label.
    pub group: String,
    pub dynamic_range_db: f32,
    pub bands_db: [f32; 25],
    /// Analysis sample rate (Hz); `None` for sounds saved by 0.3.1 and older.
    pub sample_rate: Option<u32>,
}

/// Tabs and line breaks become spaces so a field stays on its line; ends trimmed.
pub fn clean(s: &str) -> String {
    s.replace(['\t', '\r', '\n'], " ").trim().to_owned()
}

/// Tab-separated: name, mixer, group, dynamic range, 25 bands, then the sample rate when known (29 or 30 fields).
pub fn to_line(s: &Saved) -> String {
    let mut f = vec![clean(&s.name), clean(&s.mixer), clean(&s.group)];
    f.extend([s.dynamic_range_db].iter().chain(&s.bands_db).map(|v| format!("{v:.2}")));
    f.extend(s.sample_rate.map(|r| r.to_string()));
    f.join("\t")
}

fn parse_line(line: &str) -> Option<Saved> {
    let f: Vec<&str> = line.split('\t').collect();
    let [name, mixer, group, nums @ ..] = f.as_slice() else { return None };
    let (nums, sample_rate) = match nums {
        [n @ .., r] if n.len() == 26 => (n, Some(r.trim().parse().ok()?)),
        n => (n, None),
    };
    let n: Vec<f32> = nums.iter().map(|v| v.trim().parse().ok().filter(|v: &f32| v.is_finite())).collect::<Option<_>>()?;
    if name.trim().is_empty() || n.len() != 26 {
        return None;
    }
    Some(Saved {
        name: name.to_string(),
        mixer: mixer.to_string(),
        group: group.to_string(),
        dynamic_range_db: n[0],
        bands_db: n[1..].try_into().ok()?,
        sample_rate,
    })
}

/// The sounds, and the first line (1-based) that does not parse (hand edits, truncation); those are skipped.
/// A leading BOM and blank lines are ignored; CRLF is fine.
pub fn parse(text: &str) -> (Vec<Saved>, Option<usize>) {
    let (mut list, mut bad) = (vec![], None);
    for (i, line) in text.trim_start_matches('\u{feff}').lines().enumerate().filter(|(_, l)| !l.trim().is_empty()) {
        match parse_line(line) {
            Some(s) => list.push(s),
            None => _ = bad.get_or_insert(i + 1),
        }
    }
    (list, bad)
}

/// Same name replaces in place, so indexes into the list stay valid.
pub fn upsert(list: &mut Vec<Saved>, s: Saved) {
    match list.iter_mut().find(|x| x.name == s.name) {
        Some(x) => *x = s,
        None => list.push(s),
    }
}

pub const FILE: &str = "saved.tsv";
const REMEMBER: &str = "folder.txt";

/// Folder holding `saved.tsv` unless the user picked another (see `remembered_dir`).
pub fn default_dir() -> Option<PathBuf> {
    let env = |k| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    let base = if cfg!(target_os = "macos") {
        env("HOME")?.join("Library/Application Support")
    } else if cfg!(windows) {
        env("APPDATA")?
    } else {
        env("HOME")?.join(".local/share")
    };
    Some(base.join("MixerAnalyzer"))
}

/// The folder last picked, from `folder.txt` in the default folder; blank or unreadable = None.
pub fn remembered_dir(default: &Path) -> Option<PathBuf> {
    let s = std::fs::read_to_string(default.join(REMEMBER)).ok()?;
    // Only the line break is dropped: macOS folder names may end in a space.
    Some(s.trim_end_matches(['\r', '\n'])).filter(|s| !s.trim().is_empty()).map(PathBuf::from)
}

/// Remembers `dir` as the save folder.
pub fn remember_dir(default: &Path, dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(default)?;
    std::fs::write(default.join(REMEMBER), format!("{}\n", dir.display()))
}

/// Switch to `dir`: its `saved.tsv` if there is one (as `load`), else move `current` into it.
pub fn adopt(dir: &Path, current: &[Saved]) -> std::io::Result<(Vec<Saved>, Option<usize>)> {
    let p = dir.join(FILE);
    if p.try_exists()? {
        return load(&p);
    }
    store(&p, current)?;
    Ok((current.to_vec(), None))
}

/// Shows `dir` in the file manager without waiting (explorer exits 1 even on success).
pub fn open_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let opener = if cfg!(target_os = "macos") { "open" } else if cfg!(windows) { "explorer" } else { "xdg-open" };
    std::process::Command::new(opener).arg(dir).spawn().map(drop)
}

/// Missing file = empty list; any other read error is returned so the caller never overwrites a file it could not read.
/// Lossy so one badly encoded line does not hide the rest. A bad line number (see `parse`) means the file must not be rewritten.
pub fn load(path: &Path) -> std::io::Result<(Vec<Saved>, Option<usize>)> {
    match std::fs::read(path) {
        Ok(b) => Ok(parse(&String::from_utf8_lossy(&b))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok((vec![], None)),
        Err(e) => Err(e),
    }
}

/// Writes `saved.tsv.tmp` then renames it over the file, so a crash mid-write keeps the old file.
pub fn store(path: &Path, list: &[Saved]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tsv.tmp");
    let mut f = std::fs::File::create(&tmp)?;
    f.write_all(list.iter().map(|s| to_line(s) + "\n").collect::<String>().as_bytes())?;
    f.sync_all()?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(name: &str) -> Saved {
        Saved { name: name.into(), mixer: "Mackie ProFX22v3 (2019, LEVEL SET LED)".into(), group: "CH1-12 (COMP)".into(),
                dynamic_range_db: 12.25, bands_db: std::array::from_fn(|i| -1.5 * i as f32), sample_rate: Some(48_000) }
    }
    #[test] fn line_roundtrip() {
        let s = sample("홍길동 목사님 설교");
        let line = to_line(&s);
        assert_eq!((line.split('\t').count(), line.rsplit('\t').next()), (30, Some("48000")));
        assert_eq!(parse(&line), (vec![s], None));
    }
    #[test] fn old_line_has_no_sample_rate() {
        let s = Saved { sample_rate: None, ..sample("가") };
        assert_eq!(to_line(&s).split('\t').count(), 29);
        assert_eq!(parse(&to_line(&s)), (vec![s], None));
    }
    #[test] fn bad_lines_are_skipped_and_the_first_reported() {
        let good = to_line(&sample("가"));
        let short = good.rsplitn(3, '\t').last().unwrap().to_string(); // 28 fields
        let text = format!("{good}\r\n\r\n \t \ngarbage\n{short}\n{}\n\t{}\n{}\n", good.replace("-1.50", "abc"),
                           good.split_once('\t').unwrap().1, to_line(&sample("나")));
        assert_eq!(parse(&text), (vec![sample("가"), sample("나")], Some(4)));
        assert_eq!(parse(&format!("\n{good}\n  \n\n")), (vec![sample("가")], None));
    }
    #[test] fn bom_is_stripped() {
        assert_eq!(parse(&format!("\u{feff}{}\n", to_line(&sample("가")))), (vec![sample("가")], None));
    }
    #[test] fn clean_removes_tabs_and_newlines() { assert_eq!(clean("  홍길동\t목사\r\n "), "홍길동 목사"); }
    #[test] fn upsert_replaces_same_name() {
        let mut l = vec![sample("a"), sample("b")];
        let mut b2 = sample("b"); b2.dynamic_range_db = 3.0;
        upsert(&mut l, sample("c")); upsert(&mut l, b2.clone());
        assert_eq!(l.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["a", "b", "c"]);
        assert_eq!(l[1], b2);
    }
    #[test] fn store_then_load() {
        let dir = std::env::temp_dir().join(format!("mixer-analyzer-test-{}", std::process::id()));
        let p = dir.join("MixerAnalyzer").join("saved.tsv");
        assert_eq!(load(&p).unwrap(), (vec![], None));
        let l = vec![sample("a"), sample("b")];
        store(&p, &l).unwrap();
        assert_eq!(load(&p).unwrap(), (l, None));
        std::fs::remove_dir_all(&dir).unwrap();
    }
    #[test] fn unreadable_file_is_an_error() {
        // A directory where the file should be: exists but cannot be read, on every OS and even as root.
        let p = std::env::temp_dir().join(format!("mixer-analyzer-unreadable-{}", std::process::id()));
        std::fs::create_dir_all(&p).unwrap();
        assert!(load(&p).is_err());
        assert_eq!(load(&p.join("missing.tsv")).unwrap(), (vec![], None));
        std::fs::remove_dir_all(&p).unwrap();
    }
    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("mixer-analyzer-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }
    #[test] fn remember_dir_roundtrip() {
        let d = temp("remember");
        assert_eq!(remembered_dir(&d), None);
        remember_dir(&d, Path::new("/x/y z")).unwrap();
        assert_eq!(remembered_dir(&d), Some(PathBuf::from("/x/y z")));
        remember_dir(&d, Path::new("/x/Sound ")).unwrap();
    assert_eq!(remembered_dir(&d), Some(PathBuf::from("/x/Sound ")));
    std::fs::write(d.join("folder.txt"), "  \n").unwrap();
        assert_eq!(remembered_dir(&d), None);
        std::fs::remove_dir_all(&d).unwrap();
    }
    #[test] fn adopt_reads_existing_file() {
        let d = temp("adopt-read");
        store(&d.join(FILE), &[sample("a")]).unwrap();
        assert_eq!(adopt(&d, &[sample("b")]).unwrap(), (vec![sample("a")], None));
        assert_eq!(load(&d.join(FILE)).unwrap(), (vec![sample("a")], None));
        std::fs::remove_dir_all(&d).unwrap();
    }
    #[test] fn adopt_moves_list_into_empty_folder() {
        let d = temp("adopt-move").join("new");
        let l = vec![sample("a"), sample("b")];
        assert_eq!(adopt(&d, &l).unwrap(), (l.clone(), None));
        assert_eq!(load(&d.join(FILE)).unwrap(), (l, None));
        std::fs::remove_dir_all(d.parent().unwrap()).unwrap();
    }
    #[test] fn adopt_reports_unreadable_file() {
        let d = temp("adopt-bad");
        std::fs::create_dir_all(d.join(FILE)).unwrap(); // a folder where the file should be
        assert!(adopt(&d, &[sample("a")]).is_err());
        std::fs::remove_dir_all(&d).unwrap();
    }
    #[test] fn adopt_reports_bad_line_and_keeps_file() {
        let d = temp("adopt-line");
        std::fs::create_dir_all(&d).unwrap();
        let text = format!("{}\ngarbage\n", to_line(&sample("a")));
        std::fs::write(d.join(FILE), &text).unwrap();
        assert_eq!(adopt(&d, &[sample("b")]).unwrap(), (vec![sample("a")], Some(2)));
        assert_eq!(std::fs::read_to_string(d.join(FILE)).unwrap(), text);
        std::fs::remove_dir_all(&d).unwrap();
    }
}
