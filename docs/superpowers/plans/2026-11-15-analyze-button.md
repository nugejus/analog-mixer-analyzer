# 분석 버튼과 유지되는 그래프 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 실시간 추천을 없애고, 분석 버튼(15–30초 녹음 → 한 번 분석 → 결과 유지)과 촘촘하고 유지되는 주파수 그래프로 바꾼다.

**Architecture:** `spectrum.rs`가 240점 표시와 떨어지지 않는 피크·τ 선택 EMA를 맡고, `speech.rs`의 `SpeechStats`는 녹음 하나를 통째로 받아 끝에 활성 판정·요약(표시 스펙트럼 평균 포함)을 낸다. `app.rs`가 녹음 상태(`Recording`)와 저장된 분석(`Analysis`)을 들고, 추천은 분석이나 믹서·그룹·음성이 바뀔 때만 새 `AdviceEngine`으로 계산한다.

**Tech Stack:** Rust 1.96.0, eframe/egui 0.37, egui_plot 0.38 (기존 그대로).

**Spec:** `docs/superpowers/specs/2026-11-15-analyze-button-design.md` (이 변경), 나머지는 `docs/superpowers/specs/2026-11-15-mixer-analyzer-design.md`.

## Global Constraints

- 브랜치 `feat/analyze-button`. 새 크레이트 추가 금지(dev-dependency 포함).
- `cargo test`와 `cargo clippy --all-targets -- -D warnings`가 경고 없이 통과.
- 커밋은 제목 한 줄만(본문·트레일러 없음).
- 화면 문구는 설계 문서의 한국어 문자열 그대로.
- 표시 점: f_i = 20·2^(i/24) Hz, i = 0..=239. 각 점 = f·2^(±1/48) 안의 빈 파워 합, 없으면 가장 가까운 빈.
- 목표선 = LTASS + level_offset_db − 9.03 dB, 분석 요약이 있을 때만.
- 녹음 슬라이더 15–30초 정수, 기본 20. 실시간 선 τ: 빠름 0.3 / 보통 1 (기본) / 느림 3초.
- 활성: RMS > −60 dBFS 이고 > 녹음 전체 최솟값 + 12 dB. 최소 활성 5.0초.
- 피크는 `피크 리셋`과 분석 시작 때만 리셋, 그 외에는 떨어지지 않음.
- `scripts/`, `packaging/`는 건드리지 않는다.

## Review Focus

1. 44.1/96 kHz 장치에서도 녹음이 N초 오디오 길이로 끝나야 한다 → Task 3 `recording_counts_audio_frames`.
2. 분석을 누르자마자 말을 시작해 앞쪽 무음이 없어도 말소리로 잡혀야 한다 → Task 2 `activity_judged_against_whole_recording_floor`.
3. 분석 뒤 믹서·채널그룹을 바꾸면 재녹음 없이 노브가 바뀌어야 한다 → Task 3 `analysis_advice_follows_group`.
4. 빈 간격보다 좁은 저역 점(≈300 Hz 아래)도 유한하고 해당 주파수에 소리가 보여야 한다 → Task 1 `low_points_without_bins_use_nearest_bin`.
5. 긴 쉼 뒤에도 피크 선이 떨어지지 않아야 한다 → Task 1 `peak_holds_until_reset`.

---

### Task 1: 240점 표시, 떨어지지 않는 피크, τ 선택 EMA

**Files:**
- Modify: `src/spectrum.rs` (`display_freqs`, `display_db`, `DisplayCurve`, tests)
- Modify: `src/app.rs` (호출부 2곳만: `curve.update` 인자, 목표선 오프셋)

**Interfaces:**
- Produces:
  - `pub fn display_freqs() -> Vec<f32>` (240점)
  - `pub fn display_db(bin_power: &[f32], sample_rate: f32) -> Vec<f32>`
  - `pub const DISPLAY_TO_BAND_DB: f32 = 9.03;`
  - `DisplayCurve { pub smooth_db: Vec<f32>, pub peak_db: Vec<f32> }`, `new(len)`, `update(&mut self, frame_db: &[f32], dt: f32, tau_s: f32)`, `reset_peak(&mut self)`

- [ ] **Step 1: Write the failing tests** (`peak_hold_then_decay`는 지운다)

```rust
#[test]
fn display_has_240_points_at_24th_octave() {
    let f = display_freqs();
    assert_eq!(f.len(), 240);
    assert_eq!(f[0], 20.0);
    assert!((f[24] - 40.0).abs() < 1e-3);
    assert!(*f.last().unwrap() < 20_000.0);
}
#[test]
fn display_point_holds_tone_power() {
    let sr = 48_000.0;
    let hz = 20.0 * 2f32.powf(136.0 / 24.0);
    let f = Spectrum::new(sr).push(&sine(sr, hz, 0.5, 1.0)).pop().unwrap();
    let d = display_db(&f.bin_power, sr);
    assert!((d[136] + 9.03).abs() < 1.0, "{}", d[136]);
}
#[test]
fn low_points_without_bins_use_nearest_bin() {
    let sr = 48_000.0;
    let f = Spectrum::new(sr).push(&sine(sr, 50.0, 0.5, 1.0)).pop().unwrap();
    let d = display_db(&f.bin_power, sr);
    assert!(d.iter().all(|v| v.is_finite()));
    assert!(d[32] > -15.0, "{}", d[32]); // 20·2^(32/24) ≈ 50.4 Hz
    let hi = display_freqs().iter().position(|&x| x > 200.0).unwrap();
    assert!(d[hi..].iter().all(|&v| v < -40.0));
}
#[test]
fn peak_holds_until_reset() {
    let mut d = DisplayCurve::new(1);
    d.update(&[-10.0], 0.1, 1.0);
    for _ in 0..100 {
        d.update(&[-50.0], 0.1, 1.0);
    }
    assert_eq!(d.peak_db[0], -10.0);
    d.reset_peak();
    assert_eq!(d.peak_db[0], FLOOR_DB);
    d.update(&[-50.0], 0.1, 1.0);
    assert_eq!(d.peak_db[0], -50.0);
}
#[test]
fn ema_tau_sets_speed() {
    let step = |tau: f32| {
        let mut d = DisplayCurve::new(1);
        for _ in 0..10 {
            d.update(&[-10.0], 0.1, tau);
        }
        d.smooth_db[0]
    };
    assert!((step(0.3) + 10.16).abs() < 0.1, "{}", step(0.3));
    assert!((step(3.0) + 15.47).abs() < 0.1, "{}", step(3.0));
}
```

- [ ] **Step 2: Run** `cargo test --lib spectrum` → 컴파일 실패(`reset_peak` 없음, `update` 인자 수).

- [ ] **Step 3: Implement**
  - `display_freqs`: i = 0..=239.
  - `display_db`: half = 2^(1/48). 빈 범위를 산술로: `k_lo = ceil(lo/bw)`, `k_hi = floor(hi/bw)`를 `bin_power.len()-1`로 자른다. `k_lo <= k_hi`면 그 구간 합, 아니면 `round(f/bw)` 빈(자름). 전체 스캔 금지.
  - `DisplayCurve`: `held_s`, `PEAK_HOLD_S`, `PEAK_FALL_DB_S`, `TAU_SMOOTH` 삭제. `update`는 a = 1 − e^(−dt/τ) 파워 영역 EMA, `peak = max(peak, f)`. `reset_peak`은 전부 `FLOOR_DB`.
  - `app.rs`: `curve.update(..., HOP as f32 / sr, 1.0)`, 목표선 `- 3.01` → `- spectrum::DISPLAY_TO_BAND_DB`.

- [ ] **Step 4: Run** `cargo test` 와 `cargo clippy --all-targets -- -D warnings` → 통과.

- [ ] **Step 5: Commit** `git commit -am "feat: 1/24-octave graph with held peak and EMA speed"`

---

### Task 2: 녹음 전체로 말소리 요약

**Files:**
- Modify: `src/speech.rs` (`SpeechStats`, `Summary`, tests)
- Modify: `src/advice.rs` (`KEEP_TALKING` 문구, 테스트의 `Summary` 리터럴과 문구 assert)
- Modify: `src/app.rs` (호출부 1곳: `speech.push`에 표시 스펙트럼 전달)

**Interfaces:**
- Consumes: `spectrum::{Frame, db, display_db}` (Task 1)
- Produces:
  - `SpeechStats::new(hop_seconds: f32)`: 창 없음, 받은 프레임을 전부 보관.
  - `push(&mut self, frame: &Frame, bands_db: &[f32; 25], display_db: &[f32])`
  - `frames(&self) -> usize`
  - `summary(&self) -> Option<Summary>`: 끝에 판정. 바닥 = 전체 RMS 최솟값. 활성 = RMS > −60 && > 바닥+12. 활성 < 5.0초면 None.
  - `Summary`에 `pub display_db: Vec<f32>` 추가: 활성 프레임 `display_db`의 파워 평균, 길이는 들어온 벡터 길이.

- [ ] **Step 1: Write the failing tests** (`only_last_4s_counts` 삭제. `feed`는 `display: &[f32]` 인자를 더 받는다. `const HOP_S: f32 = 4096.0 / 48_000.0;`)

```rust
#[test] fn silence_gives_none() {
    // 100 프레임 FLOOR_DB → None (기존과 같음, display는 &[])
}
#[test] fn speech_after_quiet_gives_summary() {
    // 20×(−70 rms, −60 peak, band0 −90) 다음 60프레임 rms −20/−30 번갈아, peak −6, band0 −10
    // noise_floor −70, peak −6, bands[0] −10, rms = 10·log10(0.011/2) ±0.1,
    // snr = rms+70, crest = peak−rms, active_seconds = 60·HOP_S
}
#[test] fn under_5s_of_speech_gives_none() {
    // 20 quiet + 58 active(4.95 s) → None
}
#[test] fn activity_judged_against_whole_recording_floor() {
    // 60×(−20 rms) 먼저, 뒤에 20×(−70) → Some, active_seconds = 60·HOP_S ±0.01
}
#[test] fn bands_are_power_averaged() {
    // 20 quiet, 30×band0 −10, 30×band0 −20 → bands[0] = 10·log10(0.11/2) ±0.05
}
#[test] fn display_is_power_averaged_over_active_frames() {
    // 20 quiet display [−90,−90], 30 active [−10,−30], 30 active [−20,−30]
    // → display_db[0] = 10·log10(0.055) ±0.05, display_db[1] = −30 ±0.05
}
#[test] fn dynamic_range_is_p90_minus_p10() {
    // 10 quiet −70, 61 active rms −40 + 0.5·i (i=0..=60) → 24 ±1.5
}
```

`advice.rs` 테스트: `summary()` 리터럴에 `display_db: vec![]`, `"말씀을 계속해 주세요"` assert를 `"말소리가 부족합니다"`로.

- [ ] **Step 2: Run** `cargo test --lib speech` → 컴파일 실패(`push` 인자 수, `display_db` 필드).

- [ ] **Step 3: Implement**
  - 프레임은 `(rms_db, peak_db, bands_db, display_db)`로 보관. `push`에서 활성 판정하지 않는다.
  - `MIN_ACTIVE_S = 5.0`.
  - `advice.rs`의 `KEEP_TALKING` = `말소리가 부족합니다. [분석]을 다시 누르고 녹음이 끝날 때까지 평소처럼 말씀해 주세요.`
  - `app.rs` `pump`: 프레임마다 `display_db`를 한 번 계산해 `speech.push`와 `curve.update`에 함께 쓴다. (Task 3 전까지 `Live.speech`가 무한히 쌓이는 건 괜찮다. Task 3에서 녹음으로 바뀐다.)

- [ ] **Step 4: Run** `cargo test` 와 clippy → 통과.

- [ ] **Step 5: Commit** `git commit -am "feat: analyze whole recording for speech stats"`

---

### Task 3: 분석 버튼, 유지되는 결과, 그래프 옵션, README

**Files:**
- Modify: `src/app.rs`
- Modify: `README.md` ("화면 읽는 법" 절)

**Interfaces:**
- Consumes: Task 1 `DisplayCurve::{update(.., tau_s), reset_peak}`, `DISPLAY_TO_BAND_DB`. Task 2 `SpeechStats::{new, push, frames, summary}`, `Summary.display_db`. 기존 `AdviceEngine::advise`, `Inputs`, `LevelMeter::{clipped_within, silent_for}`, `MixerFactory`, `ltass_db`.
- Produces (app.rs 안):
  - `struct Recording { stats: SpeechStats, needed: usize, howl: Option<f32> }`
    - `fn new(seconds: u32, hop_s: f32) -> Self`: needed = round(seconds / hop_s)
    - `fn push(&mut self, f: &Frame, bands: &[f32; 25], disp: &[f32], howl: Option<f32>)`: howl이 Some이면 기억
    - `fn progress(&self) -> f32` (0–1), `fn done(&self) -> bool`
  - `struct Analysis { summary: Option<Summary>, seconds: u32, clipped: bool, howl: Option<f32>, silent: bool }`
  - `fn advise(a: &Analysis, group: &ChannelGroup, voice: Voice) -> AdviceSet`: 매번 `AdviceEngine::new()`
  - `const SMOOTHING: [(&str, f32); 3] = [("빠름 0.3초", 0.3), ("보통 1초", 1.0), ("느림 3초", 3.0)];`
  - `Live::pump(&mut self, channel: usize, tau_s: f32)`; `Live`에서 `speech` 필드를 빼고 `rec: Option<Recording>`.

- [ ] **Step 1: Write the failing tests** (기존 `stalled_stream_reads_as_silent`는 `pump(0, 1.0)`로)

```rust
#[test]
fn recording_counts_audio_frames() {
    let f = Frame { bin_power: vec![], rms_db: -20.0, peak: 0.5 };
    let mut r = Recording::new(15, 4096.0 / 48_000.0);
    assert_eq!(r.needed, 176);
    for _ in 0..175 {
        r.push(&f, &[-30.0; 25], &[], None);
    }
    assert!(!r.done());
    r.push(&f, &[-30.0; 25], &[], None);
    assert!(r.done());
    assert_eq!(r.progress(), 1.0);
    assert_eq!(Recording::new(20, 4096.0 / 96_000.0).needed, 469);
    assert_eq!(Recording::new(20, 4096.0 / 44_100.0).needed, 215);
}
#[test]
fn recording_keeps_last_howl() {
    let f = Frame { bin_power: vec![], rms_db: -20.0, peak: 0.5 };
    let mut r = Recording::new(15, 4096.0 / 48_000.0);
    r.push(&f, &[-30.0; 25], &[], Some(2500.0));
    r.push(&f, &[-30.0; 25], &[], None);
    assert_eq!(r.howl, Some(2500.0));
}
#[test]
fn analysis_without_speech_says_too_little() {
    let a = Analysis { summary: None, seconds: 20, clipped: false, howl: None, silent: false };
    let g = &MixerFactory::create(MixerModel::MackieProFx22v3).groups[0];
    let s = advise(&a, g, Voice::Male);
    assert!(s.knobs.iter().all(|k| k.action == Action::Wait));
    assert!(s.notes.iter().any(|n| n.contains("말소리가 부족합니다")));
}
#[test]
fn analysis_advice_follows_group() {
    // summary: bands = ltass(Male) − 40 (편차 0), peak −12, rms −30, floor −80, snr 50, dr 10, crest 18, active 10, display_db vec![]
    // 같은 Analysis로 ProFX22v3 groups[0] → knobs 7개, YamahaMg06x groups[0] → 4개
}
```

- [ ] **Step 2: Run** `cargo test --bin mixer-analyzer` → 컴파일 실패(`Recording`, `Analysis`, `advise` 없음).

- [ ] **Step 3: Implement** (설계 문서 "녹음과 분석", "그래프" 절이 기준)
  - App 필드: `rec_seconds: u32`(20), `smooth: usize`(1), `analysis: Option<Analysis>`, `advice: Option<AdviceSet>`. `engine`, `next_advice`, `ADVICE_EVERY`, `reset_advice` 삭제. `readvise()`가 `analysis`로 `advice`를 다시 만든다. 믹서·그룹·음성이 바뀌면 호출.
  - 상단: `녹음` 슬라이더(15..=30, 접미사 `초`), `분석`/`취소` 버튼, `실시간 선` 콤보, `피크 리셋`(기존 `클리핑 리셋` 유지).
  - `분석`: `live.rec = Some(Recording::new(rec_seconds, HOP/sr))`, `live.curve.reset_peak()`. `취소`나 분석채널 변경: `live.rec = None`.
  - 녹음이 `done()`이면 꺼내서 `Analysis { summary: stats.summary(), seconds, clipped: levels.clipped_within(N), howl, silent: levels.silent_for(N) || 스트림 정지(last_data > 3초) }` 저장 후 `readvise()`.
  - 하단: 녹음 중이면 `ProgressBar`에 `녹음 중 {경과:.0}/{N}초 — 계속 말씀하세요`, `live.silent()`면 그 아래 NO_INPUT 문구(advice의 상수를 pub으로). 분석이 있으면 `노브 추천 — 말소리 {활성초:.1}초 분석` + 노브 + 안내. 없으면 `[분석]을 누르고 {N}초 동안 평소처럼 말씀하세요.` 녹음 중에도 이전 결과는 보인다.
  - 그래프: `분석 결과`(GREEN, 굵기 2.5, `summary.display_db`), `현재 소리`(BLUE), `피크`(PEACH), `목표`(점선, 요약 있을 때만, `ltass + level_offset_db − DISPLAY_TO_BAND_DB`).
  - README "화면 읽는 법": 분석 버튼·녹음 시간·결과 유지, 분석 결과/현재 소리/피크/목표 선, 실시간 선 속도, 피크 리셋, 말소리 5초 기준. 1.5초 문장 삭제.

- [ ] **Step 4: Run** `cargo test`, `cargo clippy --all-targets -- -D warnings`, 그리고 `cargo build && perl -e 'alarm 8; exec @ARGV' ./target/debug/mixer-analyzer --demo 2>run.log; grep -c panicked run.log` → 종료 코드 142, 0. `run.log` 삭제.

- [ ] **Step 5: Commit** `git add src/app.rs README.md && git commit -m "feat: analyze button replaces live advice"`
