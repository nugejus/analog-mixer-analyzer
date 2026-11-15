# 그래프 정리, 추천 세팅, 세팅 복구 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 그래프 해상도·표시 선택·노브 구간을 넣고, 역할별 추천 세팅을 목표로 고르게 하고, 전문가가 맞춘 소리를 저장해 나중에 노브 회전으로 되돌리게 한다.

**Architecture:** `spectrum.rs`는 옥타브당 점 수 n을 받아 표시 점을 만들고, `speech.rs` 요약은 활성 프레임의 빈 파워 평균을 들고 있어 해상도를 바꿔도 다시 그린다. `target.rs`가 프리셋 표와 채널그룹에 맞춘 목표를, `advice.rs`가 스위치 응답 반영과 복구 모드를, 새 `saved.rs`가 저장 파일을 맡는다. `app.rs`는 고른 목표를 `Goal{db, notes, restore_dr}`로 만들어 엔진과 그래프에 같이 넘긴다.

**Tech Stack:** Rust 1.96.0, eframe/egui 0.37, egui_plot 0.38 (기존 그대로).

**Spec:** `docs/superpowers/specs/2026-11-15-targets-and-restore-design.md` (이 변경). 나머지는 `2026-11-15-analyze-button-design.md`, `2026-11-15-mixer-analyzer-design.md`.

## Global Constraints

- 브랜치 `feat/targets-restore`. 새 크레이트 금지(dev-dependency 포함).
- `cargo test`와 `cargo clippy --all-targets -- -D warnings`가 경고 없이 통과.
- 커밋은 제목 한 줄만(본문·트레일러 없음).
- 화면 문구는 설계 문서의 한국어 문자열 그대로.
- 해상도: `1/12옥타브` `1/24옥타브` `1/48옥타브`(기본) `1/96옥타브`. 점 f_i = 20·2^(i/n), i = 0..=floor(n·log2(1000)) → 120/240/479/957점. 각 점 = f·2^(±1/(2n)) 안 빈 파워 합, 없으면 가장 가까운 빈.
- 목표선 이동 = 10·log10(W3 / max(Wn, sr/8192)), Wn = hz·(2^(1/(2n)) − 2^(−1/(2n))). sr은 **분석했을 때의** 샘플레이트.
- 피크 선 색 `mocha::SURFACE2`, 다른 선보다 먼저 그린다.
- move 식: hpf(fc) = −10·log10(1+(fc/f)^4); low_shelf(fc,g) = g/(1+(f/fc)²); high_shelf(fc,g) = g/(1+(fc/f)²); bell(fc,g,w) = g·exp(−log2(f/fc)²/(2σ²)), σ = w/(2√(2ln2)), w=0이면 σ=0.6.
- bell이 남는 조건: 스윕 MID lo ≤ hz ≤ hi, 또는 고정 MID f0에서 |log2(hz/f0)| ≤ 0.7. 셸프는 해당 셸프가 있을 때만. hpf는 항상(그룹에 `LowCut{hz}`가 있으면 그 hz로).
- 복구 모드만: 저역컷 판단 값 = mean(d[0..=1]) − mean(d[3..=4]) (63·80 Hz − 125·160 Hz, c0 대신), > 3 → `켜기 권장`, < −3 → `끄기 권장`; LOW 부스트 상한 15 dB, COMP는 (지금 DR − 저장 DR) > 4 → +30°, < −4 → −30°.
- 저장 파일: macOS `$HOME/Library/Application Support/MixerAnalyzer/saved.tsv`, Windows `%APPDATA%\MixerAnalyzer\saved.tsv`, 그 밖 `$HOME/.local/share/MixerAnalyzer/saved.tsv`. 탭 구분 29칸: 이름, 믹서, 채널그룹, dynamic_range_db, 밴드 25개(`{:.2}`).
- `scripts/`, `packaging/`는 건드리지 않는다.

## Review Focus

1. 분석 뒤 48 kHz → 96 kHz 장치로 바꿔도 남아 있는 분석 결과 선과 목표선이 제 주파수에 있어야 한다 → Task 1 `result_uses_analysis_sample_rate`.
2. 윈도에서 열어 고친(CRLF) 저장 파일이나 깨진 줄이 있어도 나머지 저장본은 읽혀야 한다 → Task 5 `bad_lines_are_skipped`.
3. 지금 목표로 고른 저장본과 같은 이름으로 다시 저장하면 그 자리에서 바뀌고 목표 번호가 어긋나지 않아야 한다 → Task 5 `upsert_replaces_same_name`.
4. 75/80 Hz 스위치 믹서에서 프리셋을 고르고 스위치를 켜면 추천이 가라앉아야 하고, 스위치를 켜라면서 LOW도 깎으라고 하면 안 된다 → Task 4 `preset_on_75hz_switch_settles_when_on`, `presets_do_not_chase_switch_with_low`.
5. 스윕 MID가 둘인 Signature 22에서 노브 구간 이름(MID 1/MID 2)과 추천 FREQ 표시가 맞는 막대에 붙어야 한다 → Task 2 `knob_spans_fixed_mid_and_two_sweeps`.

---

### Task 1: 해상도 선택과 빈 파워 평균 요약

**Files:**
- Modify: `src/spectrum.rs` (`display_freqs`, `display_db`, `display_to_band_db`, tests)
- Modify: `src/speech.rs` (`Summary`, `SpeechStats::push`, tests)
- Modify: `src/app.rs` (`Recording::push`, `Live::pump`, `Analysis`, 해상도 콤보, 그래프 호출, tests)
- Modify: `src/advice.rs` (테스트의 `Summary` 리터럴만)

**Interfaces:**
- Produces:
  - `pub fn display_freqs(per_oct: usize) -> Vec<f32>`
  - `pub fn display_db(bin_power: &[f32], sample_rate: f32, per_oct: usize) -> Vec<f32>`
  - `pub fn display_to_band_db(hz: f32, sample_rate: f32, per_oct: usize) -> f32`
  - `Summary.bin_power: Vec<f32>` (`display_db` 대신; 활성 프레임의 빈별 선형 파워 평균)
  - `SpeechStats::push(&mut self, frame: &Frame, bands_db: &[f32; 25])` (프레임의 `bin_power`를 보관)
  - app.rs: `const RESOLUTIONS: [(&str, usize); 4]`, `App.res: usize`(기본 2), `Analysis.sample_rate: f32`, `Live::pump(&mut self, channel: usize, tau_s: f32, per_oct: usize)`, `fn result_db(a: &Analysis, per_oct: usize) -> Option<Vec<f32>>`, `Recording::push(&mut self, f: &Frame, bands: &[f32; 25], howl: Option<f32>)`

- [ ] **Step 1: Write the failing tests**

spectrum.rs (`display_has_240_points_at_24th_octave`는 이것으로 바꾸고, 기존 테스트의 호출은 `24`를 넘긴다):
```rust
#[test]
fn display_point_counts_per_resolution() {
    for (n, len) in [(12, 120), (24, 240), (48, 479), (96, 957)] {
        let f = display_freqs(n);
        assert_eq!(f.len(), len, "{n}");
        assert_eq!(f[0], 20.0);
        assert!((f[n] - 40.0).abs() < 1e-2, "{n}");
        assert!(*f.last().unwrap() < 20_000.0);
    }
}
#[test]
fn display_of_mean_bins_equals_mean_of_displays() {
    let sr = 48_000.0;
    let a = Spectrum::new(sr).push(&sine(sr, 1000.0, 0.5, 1.0)).pop().unwrap().bin_power;
    let b = Spectrum::new(sr).push(&noise(48_000, 0.3)).pop().unwrap().bin_power;
    let mean: Vec<f32> = a.iter().zip(&b).map(|(x, y)| (x + y) / 2.0).collect();
    let (da, db_, dm) = (display_db(&a, sr, 48), display_db(&b, sr, 48), display_db(&mean, sr, 48));
    for i in 0..dm.len() {
        let want = db((10f32.powf(da[i] / 10.0) + 10f32.powf(db_[i] / 10.0)) / 2.0);
        assert!((dm[i] - want).abs() < 0.01, "{i}: {} vs {want}", dm[i]);
    }
}
```
`display_to_band_db_matches_widths`에 추가:
```rust
near(display_to_band_db(1000.0, 48000.0, 12), 6.03);
near(display_to_band_db(1000.0, 48000.0, 48), 12.05);
near(display_to_band_db(1000.0, 48000.0, 96), 15.06);
```
speech.rs (`display_is_power_averaged_over_active_frames`를 이것으로 바꾸고, `feed`의 마지막 인자는 `Frame.bin_power`로 들어간다):
```rust
#[test]
fn bin_power_is_mean_over_active_frames() {
    let mut s = SpeechStats::new(HOP_S);
    feed(&mut s, 20, -70.0, -60.0, -90.0, &[1.0, 1.0]);
    feed(&mut s, 30, -20.0, -6.0, -10.0, &[0.1, 0.001]);
    feed(&mut s, 30, -20.0, -6.0, -10.0, &[0.01, 0.001]);
    let p = s.summary().unwrap().bin_power;
    assert!((p[0] - 0.055).abs() < 1e-6 && (p[1] - 0.001).abs() < 1e-7, "{p:?}");
}
```
app.rs:
```rust
#[test]
fn result_uses_analysis_sample_rate() {
    let sr = 48_000.0;
    // testsig는 lib의 cfg(test)라 bin 테스트에서 못 쓴다: 1 kHz 사인을 직접 만든다.
    let tone: Vec<f32> = (0..48_000).map(|i| 0.5 * (std::f32::consts::TAU * 1000.0 * i as f32 / sr).sin()).collect();
    let bin_power = Spectrum::new(sr).push(&tone).pop().unwrap().bin_power;
    let a = Analysis {
        summary: Some(Summary { bands_db: [0.0; 25], bin_power, peak_db: -12.0, rms_db: -30.0, noise_floor_db: -80.0,
                                snr_db: 50.0, dynamic_range_db: 10.0, crest_db: 18.0, active_seconds: 10.0 }),
        sample_rate: sr, seconds: 20, clipped: false, howl: None, silent: false,
    };
    let d = result_db(&a, 48).unwrap();
    let freqs = spectrum::display_freqs(48);
    let top = (0..d.len()).max_by(|&i, &j| d[i].total_cmp(&d[j])).unwrap();
    assert!((freqs[top] / 1000.0).log2().abs() <= 1.0 / 48.0, "{}", freqs[top]);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test display_point_counts_per_resolution bin_power_is_mean result_uses_analysis_sample_rate`
Expected: 컴파일 실패(인자 개수, `bin_power`, `sample_rate`, `result_db` 없음).

- [ ] **Step 3: Implement**

- spectrum.rs: 세 함수에 `per_oct` 추가(식은 Global Constraints). `DisplayCurve`는 그대로.
- speech.rs: 프레임마다 `frame.bin_power.clone()`을 보관하고 요약 때 활성 프레임으로 빈별 평균. `display_db` 필드와 표시 인자는 없앤다.
- app.rs: `RESOLUTIONS = [("1/12옥타브", 12), ("1/24옥타브", 24), ("1/48옥타브", 48), ("1/96옥타브", 96)]`. 상단 줄 `실시간 선` 옆에 `해상도` 콤보. 바꾸면 `live.curve = DisplayCurve::new(display_freqs(n).len())`(녹음은 계속). `Analysis.sample_rate`는 분석이 끝날 때의 live 샘플레이트. `result_db`는 `a.summary`의 `bin_power`를 `a.sample_rate`로 `display_db`. 그래프의 x점은 `display_freqs(n)`, 목표선 이동은 `display_to_band_db(hz, a.sample_rate, n)`.
- advice.rs 테스트 헬퍼의 `display_db: vec![]` → `bin_power: vec![]`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test && cargo clippy --all-targets -- -D warnings`
Expected: 전부 PASS, 경고 없음.

- [ ] **Step 5: Commit**

```bash
git add src/spectrum.rs src/speech.rs src/app.rs src/advice.rs
git commit -m "feat: selectable graph resolution"
```

---

### Task 2: 피크 색, 표시 체크박스, 노브 구간

**Files:**
- Modify: `src/advice.rs` (`pub fn labels`)
- Modify: `src/app.rs` (`Show`, `Span`, `knob_spans`, `spectrum_plot`, tests)

**Interfaces:**
- Consumes: Task 1의 `display_freqs(n)`, `result_db`.
- Produces:
  - `pub fn labels(group: &ChannelGroup) -> Vec<Advice>` in advice.rs (= 지금의 비공개 `knobs(group, |_, _| Action::Wait)`)
  - app.rs: `struct Show { result: bool, live: bool, peak: bool, target: bool, knobs: bool }` (Default = 모두 true), `App.show: Show`
  - app.rs: `struct Span { label: String, lo: f32, hi: f32, mark: Option<f32> }`, `fn knob_spans(group: &ChannelGroup, advice: Option<&AdviceSet>) -> Vec<Span>`

- [ ] **Step 1: Write the failing tests** (app.rs)

```rust
fn spans(m: MixerModel, a: Option<&AdviceSet>) -> Vec<(String, f32, f32, Option<f32>)> {
    knob_spans(&MixerFactory::create(m).groups[0], a).into_iter().map(|s| (s.label, s.lo, s.hi, s.mark)).collect()
}
#[test]
fn knob_spans_profx22v3() {
    assert_eq!(spans(MixerModel::MackieProFx22v3, None), [
        ("저역컷 100Hz".into(), 20.0, 100.0, None), ("HIGH 12kHz".into(), 12_000.0, 20_000.0, None),
        ("MID".into(), 100.0, 8_000.0, None), ("LOW 80Hz".into(), 20.0, 80.0, None)]);
}
#[test]
fn knob_spans_mark_recommended_freq() {
    let g = &MixerFactory::create(MixerModel::MackieProFx22v3).groups[0];
    let mut knobs = advice::labels(g);
    knobs.iter_mut().find(|k| k.label == "FREQ").unwrap().action = Action::SetFreq { hz: 400.0 };
    let a = AdviceSet { knobs, notes: vec![], level_offset_db: 0.0 };
    assert_eq!(spans(MixerModel::MackieProFx22v3, Some(&a))[2].3, Some(400.0));
}
#[test]
fn knob_spans_fixed_mid_and_two_sweeps() {
    let fx12 = spans(MixerModel::MackieProFx12v3, None);
    let m = fx12.iter().find(|s| s.0 == "MID 2.5kHz").unwrap();
    assert!((m.1 - 2500.0 * 2f32.powf(-0.7)).abs() < 0.5 && (m.2 - 2500.0 * 2f32.powf(0.7)).abs() < 0.5);
    let g = &MixerFactory::create(MixerModel::SoundcraftSignature22).groups[0];
    let mut knobs = advice::labels(g);
    knobs.iter_mut().find(|k| k.label == "FREQ 2").unwrap().action = Action::SetFreq { hz: 200.0 };
    let a = AdviceSet { knobs, notes: vec![], level_offset_db: 0.0 };
    let s = spans(MixerModel::SoundcraftSignature22, Some(&a));
    let get = |l: &str| s.iter().find(|x| x.0 == l).unwrap().clone();
    assert_eq!(get("MID 1"), ("MID 1".into(), 400.0, 8_000.0, None));
    assert_eq!(get("MID 2"), ("MID 2".into(), 80.0, 2_000.0, Some(200.0)));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test knob_spans`
Expected: 컴파일 실패(`knob_spans`, `advice::labels` 없음).

- [ ] **Step 3: Implement**

- `knob_spans`: `labels(group)` 순서대로 EQ 노브만(GAIN·COMP·FREQ 라벨 제외). 범위는 spec "노브 구간". 스윕 MID의 mark = 라벨의 `MID`를 `FREQ`로 바꾼 노브가 `advice`에서 `SetFreq{hz}`이면 그 hz.
- 그래프: 피크 선 `SURFACE2`로 먼저 그림. `실시간 주파수 그래프` 제목 줄에 `표시` 체크박스 5개(`분석 결과` `현재 소리` `피크` `목표` `노브 구간`). `show.knobs`면 막대를 0 ~ −30 dB 사이에 겹치지 않게 줄마다 그리고 이름을 단다. 막대·표시는 범례에 나오지 않게(이름 없는 항목). mark가 있으면 그 hz에 세로 표시와 hz·2^(±0.7) 강조.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test && cargo clippy --all-targets -- -D warnings`
Expected: PASS, 경고 없음.

- [ ] **Step 5: Commit**

```bash
git add src/advice.rs src/app.rs
git commit -m "feat: graph toggles and knob ranges"
```

---

### Task 3: 스위치 응답 반영과 복구 모드 (엔진)

**Files:**
- Modify: `src/target.rs` (`hpf_db`)
- Modify: `src/advice.rs` (`Inputs`, `Action::SwitchOff`, `advise`, tests)
- Modify: `src/app.rs` (`advise()`의 `Inputs`, `caption`, LowCut 그림, caption 테스트)

**Interfaces:**
- Produces:
  - `pub fn hpf_db(fc: f32, hz: f32) -> f32` in target.rs
  - `Inputs.restore_dynamic_range_db: Option<f32>` (`Some(저장 DR)` = 복구 모드)
  - `Action::SwitchOff` (캡션 `끄기 권장`, SAPPHIRE, 스위치 그림은 꺼진 모양)

- [ ] **Step 1: Write the failing tests** (advice.rs; 기존 `Inputs` 리터럴엔 `restore_dynamic_range_db: None`)

```rust
fn run_t(g: &ChannelGroup, s: &Summary, target: [f32; 25], restore: Option<f32>) -> AdviceSet {
    AdviceEngine::new().advise(g, &Inputs { summary: Some(s), target: &target, clipped_recently: false, feedback_hz: None, silent: false, restore_dynamic_range_db: restore })
}
fn with(dev: impl Fn(f32) -> f32) -> [f32; 25] { let t = ltass_db(Voice::Male); std::array::from_fn(|b| t[b] + dev(BANDS_HZ[b])) }
#[test] fn hpf_db_values() {
    assert!((hpf_db(100.0, 100.0) + 3.01).abs() < 0.01 && (hpf_db(100.0, 50.0) + 12.30).abs() < 0.01 && hpf_db(100.0, 1000.0) > -0.001);
}
#[test] fn preset_hpf_switch_not_chased_by_low() {
    let a = run_t(&group(V3, 0), &summary(|_| 0.0), with(|f| hpf_db(100.0, f)), None);
    assert_eq!(act(&a, "저역컷 100Hz"), Action::SwitchOn);
    assert_eq!(act(&a, "LOW 80Hz"), Action::Ok);
}
#[test] fn switch_already_on_is_quiet() {
    let a = run_t(&group(V3, 0), &summary(|f| hpf_db(100.0, f)), with(|f| hpf_db(100.0, f)), None);
    assert!(a.knobs.iter().all(|k| k.action == Action::Ok), "{:?}", a.knobs);
}
#[test] fn normal_mode_never_switches_off() {
    assert_eq!(act(&run_t(&group(V3, 0), &summary(|f| hpf_db(100.0, f)), with(|_| 0.0), None), "저역컷 100Hz"), Action::Ok);
}
#[test] fn restore_switch_both_ways() {
    let off = run_t(&group(V3, 0), &summary(|f| hpf_db(100.0, f)), with(|_| 0.0), Some(10.0));
    assert_eq!(act(&off, "저역컷 100Hz"), Action::SwitchOff);
    assert_eq!(act(&off, "LOW 80Hz"), Action::Ok);
    let on = run_t(&group(V3, 0), &summary(|_| 0.0), with(|f| hpf_db(100.0, f)), Some(10.0));
    assert_eq!(act(&on, "저역컷 100Hz"), Action::SwitchOn);
    assert_eq!(act(&on, "LOW 80Hz"), Action::Ok);
}
#[test] fn restore_lifts_low_boost_cap() {
    let t = with(|f| 6.0 * Shape::LowShelf(80.0).response(f));
    assert!(db_of(act(&run_t(&group(FX12, 0), &summary(|_| 0.0), t, Some(10.0)), "LOW 80Hz")) >= 4.5);
    assert!(db_of(act(&run_t(&group(FX12, 0), &summary(|_| 0.0), t, None), "LOW 80Hz")) <= 3.0);
}
#[test] fn restore_comp_follows_saved_dynamic_range() {
    let mut s = summary(|_| 0.0); // dynamic_range_db 10
    s.crest_db = 7.0; s.peak_db = -10.0; // 보통 모드라면 반시계 30°
    let comp = |saved| act(&run_t(&group(V3, 0), &s, with(|_| 0.0), Some(saved)), "COMP");
    assert_eq!(comp(4.0), Action::Rotate { degrees: 30.0, db: 0.0 });
    assert_eq!(comp(16.0), Action::Rotate { degrees: -30.0, db: 0.0 });
    assert_eq!(comp(12.0), Action::Ok);
}
```
app.rs `knob_captions_use_spec_text`에 추가:
```rust
assert_eq!(text("저역컷 100Hz", Control::LowCut { hz: 100.0 }, Action::SwitchOff), ["끄기 권장"]);
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib advice && cargo test --bin mixer-analyzer knob_captions`
Expected: 컴파일 실패(`hpf_db`, `restore_dynamic_range_db`, `SwitchOff` 없음).

- [ ] **Step 3: Implement**

`advise` 순서: d 계산 → 저역컷 판단(반영 전 d, 지금 식 mean(d[0..=1]) − c0, c0 = mean(d[6..=18])) → 켜기면 d[b] += hpf_db(스위치hz, BANDS_HZ[b]), 끄기면 d[b] −= 같은 값 → 스윕 탐색과 EQ 맞춤은 고친 d로 → 나머지(GAIN·COMP·안내)는 지금처럼. 복구 모드에서만: 저역컷 판단 값을 mean(d[0..=1]) − mean(d[3..=4])로 바꾸고 켜기(> 3)·끄기(< −3) 둘 다, LOW 상한 `EQ_LIMIT_DB`, COMP는 DR 비교 규칙(크레스트/다이내믹 규칙 대신; 과부하 안내는 그대로). app.rs `advise()`는 `restore_dynamic_range_db: None`(Task 4가 `goal.restore_dr`로 바꿈).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test && cargo clippy --all-targets -- -D warnings`
Expected: PASS(기존 `rumble_recommends_lowcut`, `deadband_hysteresis` 포함), 경고 없음.

- [ ] **Step 5: Commit**

```bash
git add src/target.rs src/advice.rs src/app.rs
git commit -m "feat: low-cut aware fit and restore mode"
```

---

### Task 4: 추천 세팅 표와 목표 메뉴

**Files:**
- Modify: `src/target.rs` (`Move`, `Preset`, `PRESETS`, `move_db`, `preset_target`, tests)
- Modify: `src/app.rs` (`Goal`, `preset_goal`, `advise()`, `App.preset`, 목표 콤보, 그래프 목표선, tests)

**Interfaces:**
- Consumes: Task 3의 `hpf_db`, `Inputs.restore_dynamic_range_db`; `advice::Shape::{LowShelf, HighShelf}.response`.
- Produces:
  - target.rs: `#[derive(Clone, Copy, Debug, PartialEq)] pub enum Move { Hpf(f32), LowShelf(f32, f32), HighShelf(f32, f32), Bell(f32, f32, f32) }` (hz, gain_db, width_oct)
  - `pub struct Preset { pub category: &'static str, pub name: &'static str, pub when: &'static str, pub moves: &'static [Move] }`, `pub const PRESETS: [Preset; 6]` (순서·문구는 spec "세팅 표" 그대로, 0번 = 기본)
  - `pub fn move_db(m: &Move, hz: f32) -> f32`
  - `pub fn preset_target(p: &Preset, voice: Voice, group: &ChannelGroup) -> ([f32; 25], Vec<f32>)` (목표, 뺀 move의 hz)
  - app.rs: `struct Goal { db: [f32; 25], notes: Vec<String>, restore_dr: Option<f32> }`, `fn preset_goal(i: usize, voice: Voice, group: &ChannelGroup) -> Goal`, `fn advise(a: &Analysis, group: &ChannelGroup, goal: &Goal) -> AdviceSet` (`restore_dynamic_range_db: goal.restore_dr`, 엔진 안내 뒤에 `goal.notes`를 붙인다), `App.preset: usize`(기본 0)

- [ ] **Step 1: Write the failing tests**

target.rs:
```rust
use crate::mixers::{MixerFactory, MixerModel};
fn g(m: MixerModel) -> crate::mixers::ChannelGroup { MixerFactory::create(m).groups.remove(0) }
#[test] fn preset_table_matches_spec() {
    let c: Vec<_> = PRESETS.iter().map(|p| p.category).collect();
    assert_eq!(c, ["기본", "찬양인도자", "찬양인도자", "설교자", "보조보컬", "보조보컬"]);
    let n: Vec<_> = PRESETS.iter().map(|p| p.name).collect();
    assert_eq!(n, ["자연스러운 목소리", "또렷한 찬양 인도", "가벼운 찬양 인도", "명료한 설교", "리드 뒤로", "남성 보조보컬"]);
    assert_eq!(PRESETS[1].moves, [Move::Hpf(100.0), Move::Bell(4000.0, 3.0, 0.0), Move::HighShelf(10000.0, 2.5)]);
    assert_eq!(PRESETS[5].moves, [Move::Hpf(135.0), Move::Bell(300.0, -4.5, 0.0)]);
}
#[test] fn move_formulas() {
    let near = |a: f32, b: f32| assert!((a - b).abs() < 0.01, "{a} vs {b}");
    near(move_db(&Move::Hpf(100.0), 100.0), -3.01);
    near(move_db(&Move::LowShelf(80.0, -6.0), 80.0), -3.0);
    near(move_db(&Move::HighShelf(10000.0, 2.5), 10000.0), 1.25);
    near(move_db(&Move::Bell(1000.0, 4.0, 0.0), 2000.0), 0.997);
    near(move_db(&Move::Bell(1000.0, 4.0, 1.0), 1000.0 * 2f32.sqrt()), 2.0);
}
#[test] fn natural_preset_is_ltass() {
    assert_eq!(preset_target(&PRESETS[0], Voice::Female, &g(MixerModel::MackieProFx22v3)), (ltass_db(Voice::Female), vec![]));
}
#[test] fn hpf_uses_group_switch_hz() {
    let (t, _) = preset_target(&PRESETS[3], Voice::Male, &g(MixerModel::Mackie1202Vlz4)); // LowCut 75
    let want = ltass_db(Voice::Male)[0] + hpf_db(75.0, 63.0) + move_db(&Move::Bell(3000.0, 4.0, 0.0), 63.0);
    assert!((t[0] - want).abs() < 1e-4);
    let (t, _) = preset_target(&PRESETS[5], Voice::Male, &g(MixerModel::SoundcraftEpm)); // 스위치 없음
    assert!((t[0] - (ltass_db(Voice::Male)[0] + hpf_db(135.0, 63.0) + move_db(&Move::Bell(300.0, -4.5, 0.0), 63.0))).abs() < 1e-4);
}
#[test] fn moves_without_knob_are_dropped() {
    let dropped = |i: usize, m| preset_target(&PRESETS[i], Voice::Male, &g(m)).1;
    assert_eq!(dropped(2, MixerModel::MackieProFx12v3), [300.0]); // 고정 MID 2.5k
    assert_eq!(dropped(4, MixerModel::MackieProFx12v3), Vec::<f32>::new()); // 3750은 0.7옥타브 안
    assert_eq!(dropped(3, MixerModel::AhZedi10), [3000.0]); // 고정 MID 600
    assert_eq!(dropped(4, MixerModel::YamahaMg06x), [3750.0]); // MID 없음
    assert_eq!(dropped(2, MixerModel::MackieProFx22v3), Vec::<f32>::new());
    let (t, _) = preset_target(&PRESETS[2], Voice::Male, &g(MixerModel::MackieProFx12v3));
    let want = ltass_db(Voice::Male)[7] + hpf_db(100.0, 315.0) + move_db(&Move::HighShelf(10000.0, 2.5), 315.0);
    assert!((t[7] - want).abs() < 1e-4);
}
```
app.rs:
```rust
fn natural(dev: impl Fn(f32) -> f32) -> Analysis { /* bands = ltass(Male) − 40 + dev(BANDS_HZ[b]), 나머지는 analysis_advice_follows_group의 값, sample_rate 48000 */ }
fn knob(a: &AdviceSet, l: &str) -> Action { a.knobs.iter().find(|k| k.label == l).unwrap().action.clone() }
#[test] fn presets_do_not_chase_switch_with_low() {
    let g = &MixerFactory::create(MixerModel::MackieProFx22v3).groups[0];
    for i in 1..PRESETS.len() {
        let s = advise(&natural(|_| 0.0), g, &preset_goal(i, Voice::Male, g));
        assert_eq!(knob(&s, "저역컷 100Hz"), Action::SwitchOn, "{i}");
        assert_eq!(knob(&s, "LOW 80Hz"), Action::Ok, "{i}");
    }
}
#[test] fn preset_on_75hz_switch_settles_when_on() {
    let g = &MixerFactory::create(MixerModel::Mackie1202Vlz4).groups[0];
    let s = advise(&natural(|f| hpf_db(75.0, f)), g, &preset_goal(1, Voice::Male, g));
    assert_eq!(knob(&s, "저역컷 75Hz"), Action::Ok);
    assert_eq!(knob(&s, "LOW 80Hz"), Action::Ok);
}
#[test] fn light_leader_on_fixed_mid_does_not_brighten() {
    let g = &MixerFactory::create(MixerModel::MackieProFx12v3).groups[0];
    let s = advise(&natural(|_| 0.0), g, &preset_goal(2, Voice::Male, g));
    match knob(&s, "HIGH 12kHz") { Action::Ok => {}, Action::Rotate { db, .. } => assert!(db <= 4.0, "{db}"), x => panic!("{x:?}") }
    assert!(s.notes.iter().any(|n| n == "이 채널에는 300Hz 부근을 바꿀 노브가 없어 그 부분은 빼고 맞춥니다."));
}
#[test] fn every_preset_on_every_group_is_finite() {
    for m in MixerModel::ALL { for g in &MixerFactory::create(m).groups { for i in 0..PRESETS.len() {
        for k in advise(&natural(|_| 0.0), g, &preset_goal(i, Voice::Female, g)).knobs {
            if let Action::Rotate { degrees, db } = k.action { assert!(degrees.is_finite() && db.is_finite()); }
        }
    }}}
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test preset move_formulas natural_preset hpf_uses moves_without light_leader every_preset`
Expected: 컴파일 실패(`PRESETS`, `preset_goal` 등 없음).

- [ ] **Step 3: Implement**

- target.rs: 표와 식은 Global Constraints/spec. 셸프는 `advice::Shape::{LowShelf, HighShelf}(fc).response(hz) * g`를 쓴다(식 중복 금지).
- app.rs: `목표` 콤보에서 분류 제목(선택 불가 글자) 아래 이름들. 닫힌 글자 `{category} · {name}`. 고른 세팅의 `when`을 콤보 옆에 SUBTEXT0. 남성/여성은 그대로. `App.goal: Goal`을 프리셋·음성·믹서·채널그룹이 바뀔 때 다시 만들고 `readvise`가 그것으로 계산. 그래프 목표선은 `goal.db`에 level_offset과 해상도 보정.
- 뺀 move 안내: `이 채널에는 {fmt_hz(hz)} 부근을 바꿀 노브가 없어 그 부분은 빼고 맞춥니다.` (hz마다 한 줄)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test && cargo clippy --all-targets -- -D warnings`
Expected: PASS, 경고 없음.

- [ ] **Step 5: Commit**

```bash
git add src/target.rs src/app.rs
git commit -m "feat: church vocal presets as targets"
```

---

### Task 5: 소리 저장·복구·삭제와 README

**Files:**
- Create: `src/saved.rs`
- Modify: `src/lib.rs` (`pub mod saved;`)
- Modify: `src/app.rs` (`Pick`, `saved_goal`, 저장·삭제 UI, 음성 비활성, 안내, tests)
- Modify: `README.md` (화면 읽는 법)

**Interfaces:**
- Consumes: Task 4의 `Goal`, `advise(a, group, goal)`; Task 3의 복구 모드.
- Produces:
  - `#[derive(Clone, Debug, PartialEq)] pub struct Saved { pub name: String, pub mixer: String, pub group: String, pub dynamic_range_db: f32, pub bands_db: [f32; 25] }`
  - `pub fn clean(s: &str) -> String` (탭·CR·LF → 공백, 앞뒤 공백 제거)
  - `pub fn to_line(s: &Saved) -> String`, `pub fn parse(text: &str) -> Vec<Saved>` (칸 수·숫자·빈 이름이 틀린 줄은 건너뜀, CRLF 허용)
  - `pub fn upsert(list: &mut Vec<Saved>, s: Saved)` (같은 이름이면 **그 자리에서** 교체, 없으면 끝에 추가)
  - `pub fn default_path() -> Option<PathBuf>` (`std::env::var_os("HOME")`/`var_os("APPDATA")`, Global Constraints의 경로)
  - `pub fn load(path: &Path) -> Vec<Saved>` (없거나 못 읽으면 빈 목록), `pub fn store(path: &Path, list: &[Saved]) -> std::io::Result<()>` (부모 폴더 생성, `saved.tsv.tmp`에 쓴 뒤 rename)
  - app.rs: `enum Pick { Preset(usize), Saved(usize) }` (`App.preset` 대신), `fn saved_goal(s: &Saved, mixer: &str, group: &str) -> Goal`

- [ ] **Step 1: Write the failing tests**

saved.rs:
```rust
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
    assert!(load(&p).is_empty());
    let l = vec![sample("a"), sample("b")];
    store(&p, &l).unwrap();
    assert_eq!(load(&p), l);
    std::fs::remove_dir_all(&dir).unwrap();
}
```
app.rs:
```rust
#[test] fn saved_goal_flags_other_channel() {
    let s = Saved { name: "x".into(), mixer: "Mackie ProFX22v3 (2019, LEVEL SET LED)".into(), group: "CH1-12 (COMP)".into(), dynamic_range_db: 9.0, bands_db: [0.0; 25] };
    let same = saved_goal(&s, &s.mixer, &s.group);
    assert!(same.notes.is_empty() && same.restore_dr == Some(9.0) && same.db == s.bands_db);
    let other = saved_goal(&s, &s.mixer, "CH13-14");
    assert_eq!(other.notes, ["저장할 때와 믹서/채널이 다릅니다(저장: Mackie ProFX22v3 (2019, LEVEL SET LED) CH1-12 (COMP)). 같은 채널에서 분석해야 맞습니다."]);
}
#[test] fn restore_from_saved_says_switch_off() {
    let g = &MixerFactory::create(MixerModel::MackieProFx22v3).groups[0];
    let saved_now = natural(|_| 0.0); // 저장 때: 스위치 꺼짐
    let s = Saved { name: "x".into(), mixer: "m".into(), group: g.label.into(),
                    dynamic_range_db: 10.0, bands_db: saved_now.summary.as_ref().unwrap().bands_db };
    let a = advise(&natural(|f| hpf_db(100.0, f)), g, &saved_goal(&s, "m", g.label)); // 지금: 켜짐
    assert_eq!(knob(&a, "저역컷 100Hz"), Action::SwitchOff);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test saved restore_from_saved`
Expected: 컴파일 실패(`saved` 모듈 없음).

- [ ] **Step 3: Implement**

- saved.rs: 위 Interfaces. 숫자는 `{:.2}`.
- app.rs (spec "저장과 복구" 그대로):
  - 시작할 때 `default_path()`와 `load`. `목표` 콤보 끝에 `저장한 세팅` 분류(저장본이 있을 때). 닫힌 글자 `저장한 세팅 · {이름}`. 설명 자리 `저장: {믹서} {채널} — 저장할 때와 같은 사람·같은 마이크로 분석하세요.` 저장한 세팅이면 남성/여성 비활성.
  - `saved_goal`: db = bands, restore_dr = Some(DR), 믹서나 채널이 다르면 안내 한 줄(테스트 문구).
  - `이 소리 저장`(분석 요약이 있을 때만 켜짐) → 이름 입력칸 + `저장`/`취소`. `clean(이름)`이 비면 `저장` 꺼짐. 저장 = `upsert(Saved{clean(이름), "{brand} {name}", 그룹 라벨, summary.dynamic_range_db, summary.bands_db})` 후 `store`, 목표는 그대로 두되 다시 계산. 실패나 경로 없음 → 안내 줄에 빨간 `저장하지 못했습니다: {오류}`(경로 없음 = `저장 폴더를 찾을 수 없습니다`), 성공하면 지움.
  - `삭제`: 저장한 세팅을 고른 동안만. 첫 클릭 → `정말 삭제`, 둘째 클릭 → 지우고 `store`, 목표 `Pick::Preset(0)`. 목표를 바꾸면 확인 풀림.
- README "화면 읽는 법": 해상도, 표시 체크박스, 노브 구간 막대, 피크 선 색(어두운 회색), 목표 메뉴(분류와 세팅, 남성/여성), 저장·복구·삭제(같은 사람·같은 마이크, GAIN은 복구 안 함, 스위치 켜기/끄기 권장), 저장 파일 위치(macOS/Windows), "채널 스트립 사진도 찍어 두세요".

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test && cargo clippy --all-targets -- -D warnings`
Expected: PASS, 경고 없음.

- [ ] **Step 5: Commit**

```bash
git add src/saved.rs src/lib.rs src/app.rs README.md
git commit -m "feat: save expert sound and restore knobs"
```
