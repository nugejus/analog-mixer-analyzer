# 윈도 설치 파일과 업데이트 확인 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 윈도는 NSIS 설치 파일로 배포하고, 앱이 켤 때 드라이브의 `latest.txt`로 새 버전을 확인해 윈도는 받아서 설치, 맥은 드라이브 폴더를 연다.

**Architecture:** 새 모듈 `src/update.rs`가 `latest.txt` 해석·버전 비교와 `curl` 호출(받기, 주소 열기)을 맡는다. `src/app.rs`는 켤 때 뒤 스레드로 확인하고 결과를 매 프레임 `try_recv`로 받아 사이드 패널 맨 위에 알림·버튼을 그린다. 설치 파일은 `packaging/installer.nsi`를 `scripts/build-win.sh`가 `makensis`로 만든다.

**Tech Stack:** Rust 1.96, eframe/egui 0.37, std `process::Command` + 시스템 `curl`, NSIS 3 (`brew install makensis`), cargo-xwin.

**Spec:** `docs/superpowers/specs/2026-11-15-installer-update-design.md`

## Global Constraints

- 새 Rust 의존성 없음. HTTP는 시스템 `curl`(`std::process::Command`).
- `LATEST_URL` = `https://drive.google.com/uc?export=download&id=1794QcL_4W-HXkp3ayiL_lf5Q4qpENQ-y`
- `FOLDER_URL` = `https://drive.google.com/drive/folders/16sIClrgZI2zEmbxvKF1BxcOzVhQ9OcIM`
- 버전 기준은 `Cargo.toml` 하나(`env!("CARGO_PKG_VERSION")`). 이번 버전 `0.2.0`.
- 확인 curl: `curl -fsSL --max-time 10 <LATEST_URL>`. 받기 curl: `curl -fsSL --max-time 300 -o <dest> <url>`. 윈도에서는 `CREATE_NO_WINDOW`(0x08000000).
- 받는 위치: `std::env::temp_dir().join("MixerAnalyzer-Setup.exe")`.
- 문구(그대로): `새 버전이 있습니다: {v}`, 버튼 `업데이트`(윈도) / `받으러 가기`(맥), `업데이트 받는 중…`, `업데이트를 받지 못했습니다: {e}`, `설치 파일을 실행하지 못했습니다: {e}`, `드라이브를 열지 못했습니다: {e}`, `버전 {v}`.
- 확인 실패(인터넷 없음, curl 없음, 형식 오류, 같거나 낮은 버전)는 아무것도 표시하지 않는다. 앱은 저절로 닫히지 않는다: `업데이트` 버튼을 눌러 받기·실행이 성공했을 때만 닫는다.
- 설치: `RequestExecutionLevel user`, `$LOCALAPPDATA\Programs\MixerAnalyzer`, 한국어, 바로가기·앱 목록 이름 `믹서 분석기`, 저장 세팅 `%APPDATA%\MixerAnalyzer`는 설치·제거가 건드리지 않음, 출력 `dist/MixerAnalyzer-Setup.exe` 10,400,000바이트 미만.
- 커밋 메시지는 제목 한 줄만(본문·Co-Authored-By 없음).

## Review Focus

- 메모장으로 고친 `latest.txt`(UTF-8 BOM, CRLF, 앞뒤 공백) → 그대로 읽혀야 한다. (Task 1 `parse_ignores_bom_crlf_spaces`)
- 드라이브가 텍스트 대신 HTML 오류 페이지를 줌 → 새 버전 없음, 앱 정상. (Task 1 `parse_rejects_bad_text`)
- 버전 숫자 자리 올림(0.9.0 → 0.10.0) → 문자열 비교가 아니라 숫자 비교. (Task 1 `newer_compares_numbers`)
- 받기 실패(네트워크 끊김, 파일 잠김) → 빨간 안내, 앱은 열린 채, 버튼으로 다시 시도. (Task 2 화면 확인)
- 예전 설치 위에 다시 설치(업데이트) → 덮어쓰기, 저장 세팅 유지, 제거해도 저장 세팅 유지. (Task 3 스크립트 확인)

---

### Task 1: `update` 모듈

**Files:**
- Create: `src/update.rs`
- Modify: `src/lib.rs` (`pub mod update;`, 알파벳 순서 자리)

**Interfaces:**
- Produces:
  - `pub const VERSION: &str = env!("CARGO_PKG_VERSION");`
  - `pub const LATEST_URL: &str`, `pub const FOLDER_URL: &str` (Global Constraints 값)
  - `#[derive(Clone, Debug, PartialEq)] pub struct Latest { pub version: String, pub url: String }`
  - `pub fn parse_latest(text: &str) -> Option<Latest>` — BOM 제거 후 줄마다 trim, 빈 줄 건너뛰고 처음 두 줄: 버전(점으로 나눈 u32들)과 `https://`로 시작하는 주소. 아니면 `None`.
  - `pub fn is_newer(latest: &str, current: &str) -> bool` — 점으로 나눈 u32 목록의 사전식 비교. 어느 쪽이든 숫자가 아니면 `false`.
  - `pub fn fetch_latest() -> Option<Latest>` — `LATEST_URL`을 curl로 받아 `parse_latest`. 어떤 실패든 `None`.
  - `pub fn download(url: &str, dest: &Path) -> std::io::Result<()>` — curl 종료 코드가 0이 아니면 `io::Error::other(<curl stderr를 trim한 것, 비면 "curl 종료 코드 N">)`.
  - `pub fn open_url(url: &str) -> std::io::Result<()>` — 맥 `open`, 윈도 `explorer`, 그 밖 `xdg-open`을 `spawn`.

- [ ] **Step 1: 실패하는 테스트 작성** (`src/update.rs` 아래 `#[cfg(test)] mod tests`)

```rust
#[test]
fn newer_compares_numbers() {
    assert!(is_newer("0.3.0", "0.2.0"));
    assert!(is_newer("0.10.0", "0.9.0"));
    assert!(is_newer("1.0.0", "0.99.99"));
    assert!(!is_newer("0.2.0", "0.2.0"));
    assert!(!is_newer("0.1.0", "0.2.0"));
    assert!(!is_newer("abc", "0.2.0"));
    assert!(!is_newer("0.3.0", "x"));
}

#[test]
fn parse_reads_two_lines() {
    let l = parse_latest("0.3.0\nhttps://drive.google.com/uc?export=download&id=A\n").unwrap();
    assert_eq!(l, Latest { version: "0.3.0".into(), url: "https://drive.google.com/uc?export=download&id=A".into() });
}

#[test]
fn parse_ignores_bom_crlf_spaces() {
    let l = parse_latest("\u{feff} 0.3.0 \r\n\r\n https://x.example/a \r\n").unwrap();
    assert_eq!((l.version.as_str(), l.url.as_str()), ("0.3.0", "https://x.example/a"));
}

#[test]
fn parse_rejects_bad_text() {
    assert_eq!(parse_latest(""), None);
    assert_eq!(parse_latest("0.3.0\n"), None);
    assert_eq!(parse_latest("<!DOCTYPE html>\n<html><body>Sorry</body></html>"), None);
    assert_eq!(parse_latest("v0.3\nhttps://x.example/a"), None);
    assert_eq!(parse_latest("0.3.0\nhttp://x.example/a"), None);
}

/// Network: run with `cargo test --lib update -- --ignored`.
#[test]
#[ignore]
fn drive_latest_parses() {
    assert!(fetch_latest().is_some());
}
```

- [ ] **Step 2: 실패 확인** — `cargo test --lib update` → 컴파일 실패(함수 없음).
- [ ] **Step 3: 구현** — curl 호출은 private `fn curl() -> Command` 하나로(윈도에서 `std::os::windows::process::CommandExt::creation_flags(0x08000000)`).
- [ ] **Step 4: 통과 확인**
  - `cargo test --lib update` → 4 passed, 1 ignored
  - `cargo test --lib update -- --ignored` → `drive_latest_parses` passed (지금 드라이브 `latest.txt`는 `0.1.0` + https 주소)
  - `cargo clippy --all-targets -- -D warnings` → 경고 없음
  - `PATH=/opt/homebrew/opt/llvm/bin:$PATH cargo xwin build --release --target x86_64-pc-windows-msvc` → 성공(윈도 전용 코드 컴파일 확인)
- [ ] **Step 5: 커밋** — `git commit -m "feat: update module for latest.txt check and download"`

### Task 2: 앱에 업데이트 알림·버튼·버전 표시

**Files:**
- Modify: `src/app.rs` (`App` 구조체·`App::new`·`side_panel`)

**Interfaces:**
- Consumes: Task 1의 `update::{VERSION, FOLDER_URL, Latest, is_newer, fetch_latest, download, open_url}`.
- Produces: 없음(화면 동작만).

상태:

```rust
enum Update {
    /// Background check still running.
    Checking(mpsc::Receiver<Option<Latest>>),
    /// Nothing to show: not newer, offline, or check failed.
    Hidden,
    Found(Latest),
    Downloading(Latest, mpsc::Receiver<std::io::Result<PathBuf>>),
    Failed(Latest, String),
}
```

- [ ] **Step 1: `App`에 `update: Update` 필드.** `App::new`에서 스레드를 띄워 `update::fetch_latest().filter(|l| update::is_newer(&l.version, update::VERSION))`를 보내고 `Update::Checking(rx)`로 시작(데모 모드도 같음).
- [ ] **Step 2: 사이드 패널 맨 위(`믹서` 라벨 앞)에 알림.**
  - 매 프레임 `Checking`/`Downloading`의 `try_recv`를 확인한다(앱은 매 프레임 다시 그림). `Checking`: 받은 값 `Some(l)` → `Found(l)`, `None`이나 연결 끊김 → `Update::Hidden`.
  - `Found`/`Failed`: `새 버전이 있습니다: {v}` + 버튼. `Failed`면 그 아래 빨간 글씨(`mocha::RED`)로 오류.
  - 윈도(`cfg!(windows)`) 버튼 `업데이트` → 스레드에서 `update::download(&l.url, &temp_dir().join("MixerAnalyzer-Setup.exe"))` 후 경로를 보냄, `Downloading`. `Downloading`: `업데이트 받는 중…`(버튼 없음). 받기 `Ok(path)` → `std::process::Command::new(&path).spawn()`, 성공하면 `ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close)`, 실패 `Failed(l, "설치 파일을 실행하지 못했습니다: {e}")`. 받기 `Err` → `Failed(l, "업데이트를 받지 못했습니다: {e}")`.
  - 맥·그 밖 버튼 `받으러 가기` → `update::open_url(update::FOLDER_URL)`, 실패 `Failed(l, "드라이브를 열지 못했습니다: {e}")`.
  - 알림 뒤에 `ui.separator()`. 알림이 없으면 아무것도 그리지 않는다.
- [ ] **Step 3: 사이드 패널 맨 아래(`folder_controls` 뒤)에** `ui.separator()`와 `ui.colored_label(mocha::SUBTEXT0, format!("버전 {}", update::VERSION))`.
- [ ] **Step 4: 확인**
  - `cargo test` → 전부 통과(기존 lib·bin 테스트 + Task 1)
  - `cargo clippy --all-targets -- -D warnings` → 경고 없음
  - `PATH=/opt/homebrew/opt/llvm/bin:$PATH cargo xwin build --release --target x86_64-pc-windows-msvc` → 성공
- [ ] **Step 5: 커밋** — `git commit -m "feat: update notice, install button, version label"`

### Task 3: 설치 파일, 빌드 스크립트, 버전 0.2.0, README

**Files:**
- Create: `packaging/installer.nsi`
- Modify: `scripts/build-win.sh`, `scripts/build-mac.sh`, `Cargo.toml` (version), `Cargo.lock`(빌드가 갱신), `README.md`

**Interfaces:**
- Consumes: `dist/MixerAnalyzer.exe`(build-win.sh가 복사), `README.md`, `assets/Pretendard-LICENSE.txt`.
- Produces: `dist/MixerAnalyzer-Setup.exe`, `dist/latest.txt`(설치 파일 ID를 알 때만).

- [ ] **Step 1: 도구 설치** — `brew install makensis sevenzip`(`sevenzip`은 확인용 `7zz`).
- [ ] **Step 2: `packaging/installer.nsi`** — `makensis -DVERSION=<x.y.z>`로 받는 NSIS 3 MUI2 스크립트.
  - `Unicode true`, `SetCompressor /SOLID lzma`, `RequestExecutionLevel user`, `Name "믹서 분석기"`, `OutFile` = `dist/MixerAnalyzer-Setup.exe`, `InstallDir "$LOCALAPPDATA\Programs\MixerAnalyzer"`.
  - 페이지: `MUI_PAGE_WELCOME`, `MUI_PAGE_INSTFILES`, `MUI_PAGE_FINISH`(`MUI_FINISHPAGE_RUN` = `$INSTDIR\MixerAnalyzer.exe`, 문구 `믹서 분석기 실행`); 제거 `MUI_UNPAGE_CONFIRM`, `MUI_UNPAGE_INSTFILES`; `MUI_LANGUAGE "Korean"`.
  - 설치: `MixerAnalyzer.exe`, `README.md`, `Pretendard-LICENSE.txt`, `Uninstall.exe`; 바로가기 `$SMPROGRAMS\믹서 분석기.lnk`, `$DESKTOP\믹서 분석기.lnk`; `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\MixerAnalyzer`에 `DisplayName "믹서 분석기"`, `DisplayVersion ${VERSION}`, `DisplayIcon`, `InstallLocation`, `UninstallString`, `NoModify 1`, `NoRepair 1`.
  - 제거: 위 파일·바로가기·레지스트리 키를 지우고 `RMDir "$INSTDIR"`(재귀 아님). `$APPDATA\MixerAnalyzer`는 건드리지 않는다.
- [ ] **Step 3: `scripts/build-win.sh`**
  - `VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)`
  - 지금처럼 exe 빌드·`dist/MixerAnalyzer.exe` 복사 → `makensis -DVERSION="$VERSION" packaging/installer.nsi` → `dist/MixerAnalyzer-Setup.exe`.
  - `dist/MixerAnalyzer-win.zip`은 지우고 더 만들지 않는다.
  - `SETUP_ID=` 변수(지금은 비어 있음, 첫 업로드 뒤 채움). `rm -f dist/latest.txt` 후, 비어 있지 않으면 `printf '%s\nhttps://drive.google.com/uc?export=download&id=%s\n' "$VERSION" "$SETUP_ID" > dist/latest.txt`, 비어 있으면 stderr에 `SETUP_ID가 비어 있어 latest.txt를 만들지 않습니다`.
- [ ] **Step 4: `scripts/build-mac.sh`** — 같은 `VERSION` 줄. 복사한 `dist/MixerAnalyzer.app/Contents/Info.plist`의 `CFBundleShortVersionString`, `CFBundleVersion`을 `plutil -replace ... -string "$VERSION"`으로 채운 뒤 서명.
- [ ] **Step 5: `Cargo.toml` `version = "0.2.0"`.**
- [ ] **Step 6: README**
  - 설치 → Windows: `MixerAnalyzer-Setup.exe`를 받아 실행하면 설치되고, 시작 메뉴·바탕화면의 **믹서 분석기**로 켭니다. 제거는 설정 → 앱 → **믹서 분석기**. 저장한 세팅은 제거해도 남습니다.
  - 첫 실행 → Windows 1번: 설치 파일을 처음 실행할 때 "Windows의 PC 보호" 창 → **추가 정보 → 실행**.
  - 새 절 `## 업데이트`(설치 절 다음): 앱을 켤 때 새 버전을 확인해 왼쪽 패널 맨 위에 **새 버전이 있습니다**가 뜹니다. Windows는 **업데이트**를 누르면 받아서 설치 화면을 열고 앱이 닫힙니다(방송 중에는 누르지 마세요). Mac은 **받으러 가기**로 드라이브 폴더를 열어 새 zip으로 앱을 바꿉니다. 인터넷이 없으면 아무것도 뜨지 않습니다. 지금 버전은 왼쪽 패널 맨 아래에 있습니다.
  - 라이선스 고지: `zip의` → `설치 폴더나 zip의`.
- [ ] **Step 7: 확인**
  - `scripts/build-win.sh` → 성공, stderr에 `SETUP_ID가 비어 있어 latest.txt를 만들지 않습니다`
  - `file dist/MixerAnalyzer-Setup.exe` → `PE32 executable ... Nullsoft Installer`; `stat -f %z dist/MixerAnalyzer-Setup.exe` < 10400000
  - `7zz l dist/MixerAnalyzer-Setup.exe` → `MixerAnalyzer.exe`, `README.md`, `Pretendard-LICENSE.txt`, `Uninstall.exe` 보임; `7zz e -so dist/MixerAnalyzer-Setup.exe MixerAnalyzer.exe | cmp - dist/MixerAnalyzer.exe` → 같음
  - `ls dist/MixerAnalyzer-win.zip` → 없음
  - `scripts/build-mac.sh` → 성공; `plutil -extract CFBundleShortVersionString raw dist/MixerAnalyzer.app/Contents/Info.plist` → `0.2.0`; `codesign -v dist/MixerAnalyzer.app` → 통과
  - `cargo test`, `cargo clippy --all-targets -- -D warnings` → 통과, 경고 없음
- [ ] **Step 8: 커밋** — `git add packaging/installer.nsi scripts/build-win.sh scripts/build-mac.sh Cargo.toml Cargo.lock README.md && git commit -m "feat: NSIS installer, version 0.2.0, update docs"`
