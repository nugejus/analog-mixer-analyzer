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
}

/// Tabs and line breaks become spaces so a field stays on its line; ends trimmed.
pub fn clean(s: &str) -> String {
    s.replace(['\t', '\r', '\n'], " ").trim().to_owned()
}

/// 29 tab-separated fields: name, mixer, group, dynamic range, 25 bands.
pub fn to_line(s: &Saved) -> String {
    let mut f = vec![clean(&s.name), clean(&s.mixer), clean(&s.group)];
    f.extend([s.dynamic_range_db].iter().chain(&s.bands_db).map(|v| format!("{v:.2}")));
    f.join("\t")
}

fn parse_line(line: &str) -> Option<Saved> {
    let f: Vec<&str> = line.split('\t').collect();
    let [name, mixer, group, nums @ ..] = f.as_slice() else { return None };
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
    })
}

/// Lines that do not parse (hand edits, truncation) are skipped; CRLF is fine.
pub fn parse(text: &str) -> Vec<Saved> {
    text.lines().filter_map(parse_line).collect()
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
    Some(s.trim()).filter(|s| !s.is_empty()).map(PathBuf::from)
}

/// Remembers `dir` as the save folder.
pub fn remember_dir(default: &Path, dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(default)?;
    std::fs::write(default.join(REMEMBER), format!("{}\n", dir.display()))
}

/// Switch to `dir`: its `saved.tsv` if there is one, else move `current` into it.
pub fn adopt(dir: &Path, current: &[Saved]) -> std::io::Result<Vec<Saved>> {
    let p = dir.join(FILE);
    if p.try_exists()? {
        return load(&p);
    }
    store(&p, current)?;
    Ok(current.to_vec())
}

/// Shows `dir` in the file manager without waiting (explorer exits 1 even on success).
pub fn open_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let opener = if cfg!(target_os = "macos") { "open" } else if cfg!(windows) { "explorer" } else { "xdg-open" };
    std::process::Command::new(opener).arg(dir).spawn().map(drop)
}

/// Missing file = empty list; any other read error is returned so the caller never overwrites a file it could not read.
/// Lossy so one badly encoded line does not hide the rest.
pub fn load(path: &Path) -> std::io::Result<Vec<Saved>> {
    match std::fs::read(path) {
        Ok(b) => Ok(parse(&String::from_utf8_lossy(&b))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
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
                dynamic_range_db: 12.25, bands_db: std::array::from_fn(|i| -1.5 * i as f32) }
    }
    #[test] fn line_roundtrip() { let s = sample("홍길동 목사님 설교"); assert_eq!(parse(&to_line(&s)), [s]); }
    #[test] fn bad_lines_are_skipped() {
        let good = to_line(&sample("가"));
        let short = good.rsplit_once('\t').unwrap().0.to_string();
        let text = format!("{good}\r\n\r\ngarbage\n{short}\n{}\n\t{}\n", good.replace("-1.50", "abc"), good.split_once('\t').unwrap().1);
        assert_eq!(parse(&text), [sample("가")]);
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
        assert!(load(&p).unwrap().is_empty());
        let l = vec![sample("a"), sample("b")];
        store(&p, &l).unwrap();
        assert_eq!(load(&p).unwrap(), l);
        std::fs::remove_dir_all(&dir).unwrap();
    }
    #[test] fn unreadable_file_is_an_error() {
        // A directory where the file should be: exists but cannot be read, on every OS and even as root.
        let p = std::env::temp_dir().join(format!("mixer-analyzer-unreadable-{}", std::process::id()));
        std::fs::create_dir_all(&p).unwrap();
        assert!(load(&p).is_err());
        assert!(load(&p.join("missing.tsv")).unwrap().is_empty());
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
        std::fs::write(d.join("folder.txt"), "  \n").unwrap();
        assert_eq!(remembered_dir(&d), None);
        std::fs::remove_dir_all(&d).unwrap();
    }
    #[test] fn adopt_reads_existing_file() {
        let d = temp("adopt-read");
        store(&d.join(FILE), &[sample("a")]).unwrap();
        assert_eq!(adopt(&d, &[sample("b")]).unwrap(), [sample("a")]);
        assert_eq!(load(&d.join(FILE)).unwrap(), [sample("a")]);
        std::fs::remove_dir_all(&d).unwrap();
    }
    #[test] fn adopt_moves_list_into_empty_folder() {
        let d = temp("adopt-move").join("new");
        let l = vec![sample("a"), sample("b")];
        assert_eq!(adopt(&d, &l).unwrap(), l);
        assert_eq!(load(&d.join(FILE)).unwrap(), l);
        std::fs::remove_dir_all(d.parent().unwrap()).unwrap();
    }
    #[test] fn adopt_reports_unreadable_file() {
        let d = temp("adopt-bad");
        std::fs::create_dir_all(d.join(FILE)).unwrap(); // a folder where the file should be
        assert!(adopt(&d, &[sample("a")]).is_err());
        std::fs::remove_dir_all(&d).unwrap();
    }
}
