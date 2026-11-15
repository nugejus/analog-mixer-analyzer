# 저장 폴더 고르기와 읽기·쓰기 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 왼쪽 패널에 `저장 폴더` 묶음을 넣어 저장 폴더를 고르고(기억), 그 폴더의 `saved.tsv`를 읽고 쓰고, 폴더를 Finder/탐색기로 열게 한다.

**Architecture:** 파일·폴더 일은 `saved.rs`(기본 폴더, `folder.txt` 기억, 폴더 옮기기 `adopt`, 폴더 열기)가 맡고, `app.rs`는 지금 폴더·쓰기 막힘 상태와 버튼만 든다. 저장 파일 형식과 `load`/`store`는 그대로.

**Tech Stack:** Rust 1.96.0, eframe/egui 0.37, `rfd =0.17.2`(새 의존성, 폴더 선택 창).

**Spec:** `docs/superpowers/specs/2026-11-15-save-folder-design.md`

## Global Constraints

- 브랜치 `feat/side-panel`(커밋 안 된 사이드 패널 변경을 먼저 `feat: controls in a left side panel`으로 커밋하고 그 위에서). 새 크레이트는 `rfd = "=0.17.2"` 하나.
- `cargo test`와 `cargo clippy --all-targets -- -D warnings`가 경고 없이 통과.
- 커밋은 제목 한 줄만(본문·트레일러 없음).
- 화면 문구는 설계 문서의 한국어 문자열 그대로.
- 저장 파일 `<폴더>/saved.tsv`, 기억 파일 `<기본 폴더>/folder.txt`(경로 한 줄). 기본 폴더 = macOS `$HOME/Library/Application Support/MixerAnalyzer`, Windows `%APPDATA%\MixerAnalyzer`, 그 밖 `$HOME/.local/share/MixerAnalyzer`.
- 읽지 못한 파일은 자동으로 덮어쓰지 않는다(`쓰기` 버튼만 예외).
- `scripts/`, `packaging/`는 건드리지 않는다.

## Review Focus

1. 고른 폴더에 읽을 수 없는 `saved.tsv`가 있을 때 저장·삭제가 그 파일을 덮어쓰면 안 된다 → Task 2 `blocked_write_keeps_file`.
2. 빈 새 폴더를 고르면 지금 목록이 사라지지 않고 그 폴더로 옮겨져야 한다 → Task 1 `adopt_moves_list_into_empty_folder`.
3. 저장본이 있는 폴더를 고르면 그 폴더 목록이 보이고 이전 폴더 파일은 그대로여야 한다 → Task 1 `adopt_reads_existing_file`.
4. 읽기·폴더 바꾸기로 목록이 바뀌어도 고른 저장한 세팅이 다른 항목을 가리키면 안 된다 → Task 2 `repick_follows_name`.
5. `folder.txt`가 비었거나 공백뿐이면 기본 폴더를 써야 한다 → Task 1 `remember_dir_roundtrip`.

---

### Task 1: 저장 폴더 도우미 (saved.rs)

**Files:**
- Modify: `src/saved.rs`

**Interfaces:**
- Produces:
  - `pub const FILE: &str = "saved.tsv";`
  - `pub fn default_dir() -> Option<PathBuf>` (`default_path` 대신; `saved.tsv`를 붙이지 않은 폴더)
  - `pub fn remembered_dir(default: &Path) -> Option<PathBuf>` (`folder.txt` 앞뒤 공백 제거, 비면 None, 읽기 실패도 None)
  - `pub fn remember_dir(default: &Path, dir: &Path) -> std::io::Result<()>` (기본 폴더를 만들고 경로 + `\n`을 씀)
  - `pub fn adopt(dir: &Path, current: &[Saved]) -> std::io::Result<Vec<Saved>>` (`dir/saved.tsv`가 있으면 `load`, 없으면 `current`를 거기에 `store`하고 `current`를 돌려줌; 존재 확인은 `try_exists`)
  - `pub fn open_dir(dir: &Path) -> std::io::Result<()>` (`create_dir_all` 뒤 macOS `open`, Windows `explorer`, 그 밖 `xdg-open`을 `spawn`; 기다리지 않음 — explorer는 성공해도 종료 코드 1)

- [ ] **Step 1: Write the failing tests**

```rust
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib saved`
Expected: 컴파일 실패(`remembered_dir`, `remember_dir`, `adopt`, `FILE` 없음).

- [ ] **Step 3: Implement** (Interfaces 그대로; `default_path` 호출부 app.rs는 이 태스크에서 `default_dir().map(|d| d.join(FILE))`로만 바꿔 컴파일을 유지)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test && cargo clippy --all-targets -- -D warnings`
Expected: PASS, 경고 없음.

- [ ] **Step 5: Commit**

```bash
git add src/saved.rs src/app.rs
git commit -m "feat: save folder helpers"
```

---

### Task 2: 저장 폴더 묶음 (app.rs, rfd, README)

**Files:**
- Modify: `Cargo.toml`, `Cargo.lock` (`rfd = "=0.17.2"`)
- Modify: `src/app.rs`
- Modify: `README.md` ("소리 저장과 복구"의 저장 파일 위치 줄을 저장 폴더 설명으로)

**Interfaces:**
- Consumes: Task 1 전부, 기존 `saved::{load, store, Saved}`.
- Produces (app.rs):
  - 필드: `default_dir: Option<PathBuf>`, `dir: Option<PathBuf>`(`saved_path` 대신), `write_blocked: bool`. 빨간 안내는 기존 `save_error` 하나로.
  - `fn repick(pick: Pick, old: &[Saved], new: &[Saved]) -> Pick` (Preset은 그대로, Saved는 이름으로 새 번호, 없으면 `Pick::Preset(0)`)
  - `fn write_list(dir: Option<&Path>, blocked: bool, list: &[Saved]) -> Result<(), String>` (None → `저장 폴더를 찾을 수 없습니다`, blocked → `읽지 못한 파일은 덮어쓰지 않습니다. [읽기]나 [쓰기]를 누르세요.`, 아니면 `store(dir/FILE)`의 오류 문자열). `persist`는 `저장하지 못했습니다: {e}`로 감싼다.
  - `fn set_list(&mut self, list: Vec<Saved>)` (repick 후 readvise)

- [ ] **Step 1: Write the failing tests** (app.rs; `saved_sample(name)`은 이 테스트 모듈에 작은 헬퍼로)

```rust
#[test] fn repick_follows_name() {
    let (a, b, c) = (saved_sample("a"), saved_sample("b"), saved_sample("c"));
    let old = [a.clone(), b.clone()];
    assert_eq!(repick(Pick::Saved(1), &old, &[b.clone(), c.clone()]), Pick::Saved(0));
    assert_eq!(repick(Pick::Saved(0), &old, &[b, c]), Pick::Preset(0));
    assert_eq!(repick(Pick::Preset(3), &old, &[]), Pick::Preset(3));
}
#[test] fn blocked_write_keeps_file() {
    let d = std::env::temp_dir().join(format!("mixer-analyzer-blocked-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join(saved::FILE), "keep me").unwrap();
    assert_eq!(write_list(Some(d.as_path()), true, &[saved_sample("x")]),
               Err("읽지 못한 파일은 덮어쓰지 않습니다. [읽기]나 [쓰기]를 누르세요.".to_owned()));
    assert_eq!(std::fs::read_to_string(d.join(saved::FILE)).unwrap(), "keep me");
    assert_eq!(write_list(None, false, &[]), Err("저장 폴더를 찾을 수 없습니다".to_owned()));
    assert!(write_list(Some(d.as_path()), false, &[saved_sample("x")]).is_ok());
    std::fs::remove_dir_all(&d).unwrap();
}
```
(`Pick`에 `Debug`가 없으면 derive를 더한다.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --bin mixer-analyzer -- repick_follows_name blocked_write_keeps_file`
Expected: 컴파일 실패(`repick`, `write_list` 없음).

- [ ] **Step 3: Implement** (spec "동작" 그대로)

- 시작: `default_dir`, 기억한 폴더가 있고 존재하면 그것, 기억했는데 없으면 기본 폴더 + 안내 `저장 폴더를 찾을 수 없어 기본 폴더를 씁니다: {경로}`. 읽기 실패면 빈 목록, `write_blocked = true`, 안내 `읽지 못했습니다: {e}`.
- 왼쪽 패널 맨 아래 구분선 뒤 `저장 폴더`: 경로(줄임 + 마우스 올리면 전체), `찾아보기…` `폴더 열기`, `읽기` `쓰기`. 폴더가 None이면 `찾아보기…`만 켜짐.
  - `찾아보기…`: `rfd::FileDialog::new().set_directory(지금 폴더).pick_folder()`. 고르면 `dir` = 그 폴더, `default_dir`이 있으면 `remember_dir`(실패는 `저장하지 못했습니다: {e}`), `adopt` → Ok면 `set_list`, `write_blocked = false`, 안내 지움 / Err면 `write_blocked = true`, `읽지 못했습니다: {e}`.
  - `폴더 열기`: `open_dir` 실패 → `폴더를 열지 못했습니다: {e}`.
  - `읽기`: `load` → Ok면 `set_list`, 막힘 풀고 안내 지움 / Err면 막힘, `읽지 못했습니다: {e}`, 목록 그대로.
  - `쓰기`: 막힘과 상관없이 `store` → Ok면 막힘 풀고 안내 지움 / Err면 `저장하지 못했습니다: {e}`.
- `이 소리 저장`·`삭제`는 기존 `persist` → `write_list(self.dir, self.write_blocked, ..)`.
- README: 저장 파일 위치 줄을 "왼쪽 패널 **저장 폴더**에서 폴더를 고르고(다시 켜도 기억), **폴더 열기**로 Finder/탐색기에서 보고, **읽기**는 파일을 다시 읽고 **쓰기**는 지금 목록으로 덮어씁니다. 저장본이 있는 폴더를 고르면 그 목록을, 빈 폴더를 고르면 지금 목록을 그 폴더로 옮깁니다. 기본 폴더는 Mac `~/Library/Application Support/MixerAnalyzer`, Windows `%APPDATA%\MixerAnalyzer`."로.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test && cargo clippy --all-targets -- -D warnings && bash scripts/build-win.sh`
Expected: PASS, 경고 없음, 윈도 크로스 빌드 성공(rfd가 xwin으로 빌드되는지 확인).

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock src/app.rs README.md
git commit -m "feat: choose, read and write the save folder"
```
