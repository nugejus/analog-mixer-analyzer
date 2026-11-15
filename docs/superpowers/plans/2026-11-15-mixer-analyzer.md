# 믹서 분석기 구현 계획

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**목표:** 믹서 출력을 실시간으로 분석해서 다음을 보여주는 Rust 데스크톱 앱을 만듭니다.
- 주파수 그래프
- 디스토션 여부
- 선택한 믹서의 노브별 "방향 + 각도" 추천

**구조:** 순수 분석 모듈(`src/lib.rs` 아래)과 GUI 바이너리(`src/main.rs`, `src/app.rs`)로 나눕니다.
- 분석 모듈은 egui 없이 단위 테스트합니다.
- 오디오 콜백은 샘플을 rtrb 링에 넣기만 합니다. 모든 분석은 UI 스레드에서 매 프레임 합니다.
- 믹서 프로필은 `MixerFactory::create(MixerModel)`가 만드는 데이터입니다.

**기술 스택:** Rust 1.96.0, eframe/egui 0.37 (wgpu), egui_plot 0.38, cpal 0.18.2, realfft 3.5.1, rtrb 0.4.1. 디자인은 Catppuccin Mocha와 Pretendard를 씁니다.

**Spec:** `docs/superpowers/specs/2026-11-15-mixer-analyzer-design.md`. 수치, 문구, 믹서 표는 모두 이 문서에 있습니다. 구현자는 이 계획과 spec을 같이 읽습니다.

## Global Constraints
- 툴체인은 `rust-toolchain.toml`(1.96.0)이고 edition은 2024입니다. 이미 커밋되어 있습니다.
- 의존성은 Cargo.toml에 고정된 5개뿐입니다. dev-dependency를 포함해 새 크레이트는 추가하지 않습니다. 테스트 신호는 `src/testsig.rs`의 LCG/사인을 씁니다.
- 화면 문구는 spec의 한국어 문구를 그대로 씁니다.
- 모든 dB 값은 유한해야 합니다. 바닥값은 `spectrum::FLOOR_DB = -150.0`입니다. NaN이나 −inf를 화면이나 추천에 내보내지 않습니다.
- 색은 `theme::mocha` 상수만 씁니다.
- 각 Task는 끝날 때 아래 두 명령이 통과해야 합니다.
  - `cargo test`
  - `cargo clippy --all-targets -- -D warnings`
- 커밋 메시지는 제목 한 줄(`type: 요약`)만 씁니다. 본문과 Co-Authored-By 줄은 넣지 않습니다.
- 작업 브랜치는 `feat/analyzer`입니다(main에서 분기).

## Review Focus
1. **마이크 권한 거부나 케이블 빠짐으로 입력이 전부 0인 경우.** 그래프는 바닥에 머물고, "입력 신호가 없습니다"를 띄우고, 노브는 모두 대기여야 합니다. NaN이나 엉뚱한 추천이 나오면 안 됩니다. 테스트: T1 `silence_is_finite_floor`, T2 `clip_recency_reset_and_silence`, T6 `silent_input_waits`.
2. **44.1/48/96 kHz 장치, 1채널 장치.** 밴드 위치가 맞아야 하고, 분석 채널 2를 골랐는데 장치가 1채널이면 0번 채널로 대체해야 합니다. 테스트: T1 `sine_reads_rms_level_at_any_rate`, T7 `pick_channel_falls_back_to_first`.
3. **분석 중에 믹서나 채널 그룹을 바꾸는 경우.** 패닉이 없어야 하고, 노브 수가 새 그룹과 같아야 하고, 히스테리시스 상태는 초기화되어야 합니다. 테스트: T6 `group_change_resets_state`.
4. **계속 클리핑이 나는 큰 입력.** GAIN 내림 추천이 나와야 하고, EQ 값은 유한하며 ±15 dB 안이어야 합니다. 테스트: T6 `clip_turns_gain_down` 안의 유한성 단언.
5. **길게 부르는 노래 음(배음 있음, 비브라토).** 하울링으로 잡으면 안 됩니다. 테스트: T4 `voiced_vibrato_is_not_feedback`.

---

### Task 1: 밴드, 목표 곡선, 스펙트럼

**Files:**
- Create: `src/lib.rs`, `src/target.rs`, `src/spectrum.rs`, `src/testsig.rs`
- Modify: `src/main.rs`(cargo init이 만든 hello world를 그대로 둡니다. T7에서 교체합니다.)

**Interfaces:**
- Produces (`target.rs`):
  - `pub const BANDS_HZ: [f32; 25]`: 63…16000, spec 표의 순서와 같습니다.
  - `#[derive(Clone, Copy, PartialEq, Eq, Debug)] pub enum Voice { Male, Female }`
  - `pub fn ltass_db(voice: Voice) -> [f32; 25]`: spec의 Byrne 표 값입니다.
- Produces (`spectrum.rs`):
  - 상수: `pub const FFT_SIZE: usize = 8192; pub const HOP: usize = 4096; pub const FLOOR_DB: f32 = -150.0;`
  - `pub struct Frame { pub bin_power: Vec<f32>, pub rms_db: f32, pub peak: f32 }`. 필드 뜻:
    - `bin_power`: 길이 FFT_SIZE/2+1, spec 정규화를 적용한 평균제곱 값
    - `rms_db`: 이번 hop 4096샘플의 RMS(dBFS)
    - `peak`: 이번 hop의 최대 |x|
  - `pub struct Spectrum`
    - `new(sample_rate: f32) -> Self`
    - `sample_rate(&self) -> f32`
    - `push(&mut self, mono: &[f32]) -> Vec<Frame>`: 첫 프레임은 8192샘플이 모였을 때 나오고, 그 뒤로는 HOP마다 하나씩 나옵니다. 호출 사이에 버퍼를 유지합니다.
  - `pub fn db(power: f32) -> f32`: 10·log10이고 FLOOR_DB 아래로는 내려가지 않습니다.
  - `pub fn band_db(bin_power: &[f32], sample_rate: f32, lo_hz: f32, hi_hz: f32) -> f32`
  - `pub fn third_octave_db(bin_power: &[f32], sample_rate: f32) -> [f32; 25]`: 경계는 fc·2^(±1/6)입니다.
  - `pub fn display_freqs() -> Vec<f32>`: 20·2^(i/6), i = 0..=60
  - `pub fn display_db(bin_power: &[f32], sample_rate: f32) -> Vec<f32>`: 각 점은 ±1/12옥타브 안의 빈 합입니다. 빈이 없으면 가장 가까운 빈을 씁니다.
  - `pub struct DisplayCurve { pub smooth_db: Vec<f32>, pub peak_db: Vec<f32>, .. }`
    - `new(len: usize) -> Self`: 모든 값이 FLOOR_DB인 상태로 시작합니다.
    - `update(&mut self, frame_db: &[f32], dt: f32)`: 평활은 전력 지수평균 τ=0.3 s이고, 피크 홀드는 1.5 s 유지 후 12 dB/s로 떨어집니다.
- Produces (`testsig.rs`, `#[cfg(test)] pub mod testsig;`로 선언):
  - `pub fn sine(sr: f32, hz: f32, amp: f32, secs: f32) -> Vec<f32>`
  - `pub fn noise(n: usize, amp: f32) -> Vec<f32>`: LCG 1664525/1013904223, seed 12345, 균일 분포 ±amp
- `lib.rs`는 `pub mod target; pub mod spectrum; #[cfg(test)] pub mod testsig;`로 시작합니다. 이후 Task에서 한 줄씩 추가합니다.

- [ ] **Step 1: `git switch -c feat/analyzer`. 아래 테스트를 `src/spectrum.rs`의 `#[cfg(test)] mod tests`에 작성합니다(`use crate::{target::BANDS_HZ, testsig::*}`).**

```rust
#[test] fn sine_reads_rms_level_at_any_rate() {
    for sr in [44_100.0, 48_000.0, 96_000.0] {
        let f = Spectrum::new(sr).push(&sine(sr, 1000.0, 0.5, 1.0)).pop().unwrap();
        let bands = third_octave_db(&f.bin_power, sr);
        let i = BANDS_HZ.iter().position(|&b| b == 1000.0).unwrap();
        assert!((bands[i] + 9.03).abs() < 0.5, "{sr}: {}", bands[i]);
        assert!(bands.iter().all(|&b| b <= bands[i]));
        assert!((f.rms_db + 9.03).abs() < 0.1);
        assert!((f.peak - 0.5).abs() < 1e-3);
    }
}
#[test] fn noise_power_sums_to_mean_square() {
    let x = noise(48_000, 0.5);
    let ms = x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32;
    let f = Spectrum::new(48_000.0).push(&x).pop().unwrap();
    assert!((db(f.bin_power.iter().sum()) - db(ms)).abs() < 0.5);
}
#[test] fn silence_is_finite_floor() {
    let f = Spectrum::new(48_000.0).push(&vec![0.0; FFT_SIZE]).pop().unwrap();
    assert!(third_octave_db(&f.bin_power, 48_000.0).iter().all(|&b| b == FLOOR_DB));
    assert_eq!(f.rms_db, FLOOR_DB);
    assert!(display_db(&f.bin_power, 48_000.0).iter().all(|b| b.is_finite()));
}
#[test] fn one_frame_per_hop_across_chunks() {
    let mut s = Spectrum::new(48_000.0);
    let mut n = 0;
    for _ in 0..8 { n += s.push(&vec![0.1; 1024]).len(); }
    assert_eq!(n, 1);
    n += s.push(&vec![0.1; HOP]).len();
    assert_eq!(n, 2);
}
#[test] fn peak_hold_then_decay() {
    let mut d = DisplayCurve::new(1);
    d.update(&[-10.0], 0.1);
    for _ in 0..10 { d.update(&[-50.0], 0.1); }
    assert!((d.peak_db[0] + 10.0).abs() < 0.01);
    for _ in 0..14 { d.update(&[-50.0], 0.1); }
    assert!((d.peak_db[0] + 21.0).abs() < 1.5);
    assert!(d.smooth_db[0] < -40.0);
}
```
`target.rs`에도 테스트를 하나 둡니다: `ltass_db(Voice::Male)[12] == 53.1`, `ltass_db(Voice::Female)[5] == 62.2`, `BANDS_HZ[0] == 63.0 && BANDS_HZ[24] == 16000.0`.

- [ ] **Step 2: `cargo test`를 실행합니다. 정의되지 않은 항목 때문에 컴파일이 실패해야 합니다.**
- [ ] **Step 3: spec "분석 (DSP)"의 FFT, 밴드, 표시 곡선 절을 따라 구현합니다.**
  - 창 함수는 Hann(주기형)입니다.
  - realfft는 입력을 덮어쓰므로 창을 곱한 복사본을 넘깁니다.
  - 플래너는 `Spectrum` 안에 한 번만 만듭니다.
- [ ] **Step 4: `cargo test`와 clippy가 통과해야 합니다.**
- [ ] **Step 5: `git add -A && git commit -m "feat: spectrum analyzer and LTASS target"`**

### Task 2: 레벨 미터와 클리핑

**Files:** Create `src/levels.rs`. Modify `src/lib.rs`(`pub mod levels;`).

**Interfaces:**
- Consumes: `spectrum::FLOOR_DB`
- Produces:
  - `pub struct HistPoint { pub peak_db: f32, pub clipped: bool }`
  - `pub struct LevelMeter`
    - `new(sample_rate: f32, channels: usize) -> Self`
    - `push_interleaved(&mut self, data: &[f32])`
    - `peak_db(&self) -> f32`: 마지막으로 완성된 100 ms 점의 값입니다. 점이 없으면 FLOOR_DB입니다.
    - `clip_count(&self) -> u32`
    - `reset_clips(&mut self)`
    - `history(&self) -> &std::collections::VecDeque<HistPoint>`: 최대 300개
    - `clipped_within(&self, seconds: f32) -> bool`
    - `silent_for(&self, seconds: f32) -> bool`: 최근 `seconds`만큼의 점이 모두 있고, 그 점들이 전부 peak < −140 dB이면 참입니다.
- 클리핑 규칙: 채널별로 |x| ≥ 0.999가 연속될 때 그 길이가 3에 **도달하는 순간** 1회로 셉니다. 연속 길이는 push 호출 사이에도 유지합니다.

- [ ] **Step 1: 테스트를 작성합니다.**

```rust
#[test] fn clip_needs_three_in_a_row() {
    let count = |x: &[f32]| { let mut m = LevelMeter::new(48_000.0, 1); m.push_interleaved(x); m.clip_count() };
    assert_eq!(count(&[0.0, 1.0, 1.0, 1.0, 0.0]), 1);
    assert_eq!(count(&[1.0, 1.0, 0.0, 1.0, 1.0]), 0);
    assert_eq!(count(&[-0.9995, -0.9995, -0.9995]), 1);
    assert_eq!(count(&[1.0; 10]), 1);
}
#[test] fn clip_run_spans_buffers_and_channels() {
    let mut m = LevelMeter::new(48_000.0, 1);
    m.push_interleaved(&[1.0, 1.0]); m.push_interleaved(&[1.0]);
    assert_eq!(m.clip_count(), 1);
    let mut s = LevelMeter::new(48_000.0, 2);
    s.push_interleaved(&[0.0, 1.0, 0.0, 1.0, 0.0, 1.0]);
    assert_eq!(s.clip_count(), 1);
    let mut t = LevelMeter::new(48_000.0, 2);
    t.push_interleaved(&[1.0, 0.0, 0.0, 1.0, 1.0, 0.0]);
    assert_eq!(t.clip_count(), 0);
}
#[test] fn history_is_100ms_points_capped_at_30s() {
    let mut m = LevelMeter::new(48_000.0, 1);
    m.push_interleaved(&vec![0.5; 48_000]);
    assert_eq!(m.history().len(), 10);
    assert!((m.peak_db() + 6.02).abs() < 0.05);
    m.push_interleaved(&vec![0.5; 48_000 * 31]);
    assert_eq!(m.history().len(), 300);
}
#[test] fn clip_recency_reset_and_silence() {
    let mut m = LevelMeter::new(48_000.0, 1);
    m.push_interleaved(&[1.0; 3]);
    m.push_interleaved(&vec![0.0; 4800]);
    assert!(m.clipped_within(4.0));
    m.push_interleaved(&vec![0.0; 48_000 * 5]);
    assert!(!m.clipped_within(4.0));
    assert!(m.silent_for(3.0));
    m.reset_clips();
    assert_eq!(m.clip_count(), 0);
    m.push_interleaved(&vec![0.1; 4800]);
    assert!(!m.silent_for(3.0));
}
```
- [ ] **Step 2: `cargo test levels`를 실행합니다. 컴파일이 실패해야 합니다.**
- [ ] **Step 3: 구현합니다.** 100 ms 점은 `sample_rate/10` 프레임(채널 묶음)마다 하나씩 닫습니다.
- [ ] **Step 4: 테스트와 clippy가 통과해야 합니다.**
- [ ] **Step 5: `git commit -am "feat: level meter with clip detection"`(새 파일은 먼저 `git add`합니다)**

### Task 3: 활성 음성 통계

**Files:** Create `src/speech.rs`. Modify `src/lib.rs`.

**Interfaces:**
- Consumes: `spectrum::{Frame, FLOOR_DB, db}`
- Produces:
  - `pub struct Summary { pub bands_db: [f32; 25], pub peak_db: f32, pub rms_db: f32, pub noise_floor_db: f32, pub snr_db: f32, pub dynamic_range_db: f32, pub crest_db: f32, pub active_seconds: f32 }`. `Clone`과 `Debug`를 derive합니다.
  - `pub struct SpeechStats`
    - `new(hop_seconds: f32) -> Self`
    - `push(&mut self, frame: &Frame, bands_db: &[f32; 25])`
    - `summary(&self) -> Option<Summary>`
- 규칙(spec "활성 음성 판정"과 "장기 평균"):
  - 창 4 s = `round(4/hop_seconds)`프레임, 잡음 바닥 창 10 s
  - 활성 조건: rms > −60이고 rms > 바닥+12
  - 활성 시간이 1.5 s 미만이면 None
  - 밴드와 RMS는 전력 평균
  - 피크 = 활성 프레임 peak의 최댓값(dB)
  - DR = 활성 rms를 정렬했을 때 `idx = round(p·(n−1))`로 뽑은 p90 − p10
  - 크레스트 = 피크 − RMS, SNR = RMS − 바닥

- [ ] **Step 1: 테스트를 작성합니다.**

```rust
const HOP_S: f32 = 4096.0 / 48_000.0;
fn feed(s: &mut SpeechStats, n: usize, rms: f32, peak_db: f32, band0: f32) {
    for _ in 0..n {
        let mut b = [-100.0; 25]; b[0] = band0;
        s.push(&Frame { bin_power: vec![], rms_db: rms, peak: 10f32.powf(peak_db / 20.0) }, &b);
    }
}
#[test] fn silence_gives_none() {
    let mut s = SpeechStats::new(HOP_S);
    feed(&mut s, 100, FLOOR_DB, FLOOR_DB, FLOOR_DB);
    assert!(s.summary().is_none());
}
#[test] fn speech_after_quiet_gives_summary() {
    let mut s = SpeechStats::new(HOP_S);
    feed(&mut s, 20, -70.0, -60.0, -90.0);
    for i in 0..30 { feed(&mut s, 1, if i % 2 == 0 { -20.0 } else { -30.0 }, -6.0, -10.0); }
    let m = s.summary().unwrap();
    assert!((m.noise_floor_db + 70.0).abs() < 0.01);
    assert!((m.peak_db + 6.0).abs() < 0.01);
    assert!((m.bands_db[0] + 10.0).abs() < 0.01);
    assert!((m.rms_db - 10.0 * (0.011f32 / 2.0).log10()).abs() < 0.1);
    assert!((m.snr_db - (m.rms_db + 70.0)).abs() < 0.01);
    assert!((m.crest_db - (m.peak_db - m.rms_db)).abs() < 0.01);
    assert!((m.active_seconds - 30.0 * HOP_S).abs() < 0.01);
}
#[test] fn only_last_4s_counts() {
    let mut s = SpeechStats::new(HOP_S);
    feed(&mut s, 20, -70.0, -60.0, -90.0);
    feed(&mut s, 30, -20.0, -6.0, -10.0);
    feed(&mut s, 60, -70.0, -60.0, -90.0);
    assert!(s.summary().is_none());
}
#[test] fn bands_are_power_averaged() {
    let mut s = SpeechStats::new(HOP_S);
    feed(&mut s, 20, -70.0, -60.0, -90.0);
    feed(&mut s, 15, -20.0, -6.0, -10.0);
    feed(&mut s, 15, -20.0, -6.0, -20.0);
    assert!((s.summary().unwrap().bands_db[0] - 10.0 * (0.11f32 / 2.0).log10()).abs() < 0.05);
}
#[test] fn dynamic_range_is_p90_minus_p10() {
    let mut s = SpeechStats::new(HOP_S);
    feed(&mut s, 10, -70.0, -60.0, -90.0);
    for i in 0..31 { feed(&mut s, 1, -40.0 + i as f32, -6.0, -10.0); }
    assert!((s.summary().unwrap().dynamic_range_db - 24.0).abs() < 1.5);
}
```
- [ ] **Step 2: 컴파일 실패를 확인합니다.**
- [ ] **Step 3: 구현합니다.** `VecDeque<(rms_db, peak_db, active, [f32; 25])>` 하나로 10 s를 유지합니다.
- [ ] **Step 4: 테스트와 clippy가 통과해야 합니다.**
- [ ] **Step 5: `git commit -m "feat: active speech statistics"`**

### Task 4: 하울링 검출

**Files:** Create `src/feedback.rs`. Modify `src/lib.rs`.

**Interfaces:**
- Consumes: `spectrum::{Spectrum, FFT_SIZE, db}`
- Produces: `pub struct FeedbackDetector`
  - `new(sample_rate: f32) -> Self`
  - `push(&mut self, bin_power: &[f32]) -> Option<f32>`: 지금 하울링 의심이면 그 주파수(Hz)를 돌려줍니다.
- 규칙은 spec "하울링 의심"을 그대로 따릅니다: PNPR ≥ 15, PHPR ≥ 20, P > −60 dB, ±2빈, 6프레임 연속, 프레임 간 변화 ≤ 2 dB, 6프레임 동안 조건이 깨지면 해제.

- [ ] **Step 1: 테스트를 작성합니다.**

```rust
fn run(x: &[f32]) -> Vec<Option<f32>> {
    let mut sp = Spectrum::new(48_000.0);
    let mut fd = FeedbackDetector::new(48_000.0);
    sp.push(x).iter().map(|f| fd.push(&f.bin_power)).collect()
}
fn mix(a: &[f32], b: &[f32]) -> Vec<f32> { a.iter().zip(b).map(|(x, y)| x + y).collect() }
#[test] fn steady_tone_is_feedback() {
    let x = mix(&sine(48_000.0, 2500.0, 0.3, 1.5), &noise(72_000, 0.001));
    let hz = run(&x).last().copied().flatten().expect("detected");
    assert!((hz - 2500.0).abs() < 25.0);
}
#[test] fn noise_is_not_feedback() {
    assert!(run(&noise(96_000, 0.3)).iter().all(Option::is_none));
}
#[test] fn voiced_vibrato_is_not_feedback() {
    // phase(t) = 2π·120·(t − 0.05/(2π·5)·cos(2π·5·t)),  x = Σ_{h=1..10} (0.1/h)·sin(h·phase)
    let x = voiced(48_000.0, 2.0);   // 테스트 안 헬퍼, 위 식
    assert!(run(&x).iter().all(Option::is_none));
}
#[test] fn released_after_tone_stops() {
    let mut x = mix(&sine(48_000.0, 2500.0, 0.3, 1.5), &noise(72_000, 0.001));
    x.extend(noise(48_000, 0.001));
    assert_eq!(*run(&x).last().unwrap(), None);
}
```
- [ ] **Step 2: 컴파일 실패를 확인합니다.**
- [ ] **Step 3: 구현합니다.** 검색 범위는 100 Hz–10 kHz의 빈입니다. 2k와 3k가 나이퀴스트를 넘으면 PHPR은 통과로 봅니다.
- [ ] **Step 4: 테스트와 clippy가 통과해야 합니다.**
- [ ] **Step 5: `git commit -m "feat: feedback (howl) detector"`**

### Task 5: 믹서 프로필 팩토리

**Files:** Create `src/mixers.rs`. Modify `src/lib.rs`.

**Interfaces:**
- Produces:
  - `#[derive(Clone, Copy, Debug, PartialEq)] pub enum Control { Gain { range_db: f32 }, LowCut { hz: f32 }, HighShelf { hz: f32 }, Mid { lo_hz: f32, hi_hz: f32 }, LowShelf { hz: f32 }, Comp }`. `lo_hz == hi_hz`면 고정 MID입니다.
  - `pub struct ChannelGroup { pub label: &'static str, pub controls: Vec<Control> }`
  - `pub struct MixerProfile { pub brand: &'static str, pub name: &'static str, pub groups: Vec<ChannelGroup>, pub pc_tip: &'static str }`
  - `#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum MixerModel { .. }`: spec 표의 32개 항목입니다. 같은 칸에 "/"로 묶인 모델은 한 항목이지만, Behringer X2222USB/QX2222USB/X2442USB는 각각 따로 셉니다.
  - `impl MixerModel { pub const ALL: [MixerModel; 32] }`
  - `pub struct MixerFactory; impl MixerFactory { pub fn create(model: MixerModel) -> MixerProfile }`
  - 상수: `pub const EQ_RANGE_DB: f32 = 15.0; pub const KNOB_TRAVEL_DEG: f32 = 300.0;`
- 컨트롤 순서: `[Gain, LowCut?, HighShelf, Mid…(높은 대역부터), LowShelf, Comp?]`
- 이름(`name`)에 세대 구분 단서를 넣습니다. 예: `"ProFX22v3 (2019, LEVEL SET LED)"`, `"ProFX22v2 (2015, VITA 프리앰프)"`, `"ProFX22 (1세대, 2011)"`.
- 문서끼리 값이 다른 항목에는 `// 신뢰도 중간: …` 주석을 답니다.
- 팁 문구는 spec 원문 그대로 `const`로 둡니다.

- [ ] **Step 1: 테스트를 작성합니다.**

```rust
use Control::*;
fn has(g: &ChannelGroup, c: Control) -> bool { g.controls.contains(&c) }
#[test] fn every_model_builds_a_sane_profile() {
    let mut names = std::collections::HashSet::new();
    for m in MixerModel::ALL {
        let p = MixerFactory::create(m);
        assert!(names.insert(format!("{} {}", p.brand, p.name)));
        assert!(!p.groups.is_empty() && !p.pc_tip.is_empty());
        for g in &p.groups { assert!(matches!(g.controls[0], Gain { .. }), "{}", p.name); }
        let labels: std::collections::HashSet<_> = p.groups.iter().map(|g| g.label).collect();
        assert_eq!(labels.len(), p.groups.len());
    }
}
#[test] fn profx22v3_strip() {
    let p = MixerFactory::create(MixerModel::MackieProFx22v3);
    assert_eq!(p.groups[0].label, "CH1-12 (COMP)");
    for c in [Gain { range_db: 60.0 }, LowCut { hz: 100.0 }, HighShelf { hz: 12_000.0 },
              Mid { lo_hz: 100.0, hi_hz: 8_000.0 }, LowShelf { hz: 80.0 }, Comp] {
        assert!(has(&p.groups[0], c), "{c:?}");
    }
    assert!(!has(&p.groups[1], Comp));
}
#[test] fn mgp24x_strip() {
    let p = MixerFactory::create(MixerModel::YamahaMgp24x);
    assert_eq!(p.groups.iter().map(|g| g.label).collect::<Vec<_>>(), ["CH1-8", "CH9-16 (COMP)"]);
    for c in [Gain { range_db: 44.0 }, LowCut { hz: 100.0 }, HighShelf { hz: 8_000.0 },
              Mid { lo_hz: 250.0, hi_hz: 5_000.0 }, LowShelf { hz: 125.0 }] {
        assert!(has(&p.groups[0], c) && has(&p.groups[1], c), "{c:?}");
    }
    assert!(!has(&p.groups[0], Comp) && has(&p.groups[1], Comp));
    assert!(p.pc_tip.contains("MONITOR OUT"));
}
#[test] fn odd_strips() {
    let sig = MixerFactory::create(MixerModel::SoundcraftSignature22);
    assert!(has(&sig.groups[0], Mid { lo_hz: 80.0, hi_hz: 2_000.0 }) && has(&sig.groups[0], Mid { lo_hz: 400.0, hi_hz: 8_000.0 }));
    let no_cut = |m| !MixerFactory::create(m).groups[0].controls.iter().any(|c| matches!(c, LowCut { .. }));
    let no_mid = |m| !MixerFactory::create(m).groups[0].controls.iter().any(|c| matches!(c, Mid { .. }));
    assert!(no_cut(MixerModel::SoundcraftEpm));
    assert!(no_mid(MixerModel::YamahaMg06x) && no_mid(MixerModel::AhZed6));
    assert!(has(&MixerFactory::create(MixerModel::MackieProFx22).groups[2], Mid { lo_hz: 2_500.0, hi_hz: 2_500.0 }));
    assert!(has(&MixerFactory::create(MixerModel::AhZedi10).groups[0], Mid { lo_hz: 600.0, hi_hz: 600.0 }));
}
```
- [ ] **Step 2: 컴파일 실패를 확인합니다.**
- [ ] **Step 3: spec "믹서 데이터" 표를 그대로 옮깁니다.** 반복을 줄이는 작은 헬퍼 fn(예: `strip(gain, cut, high, mids, low, comp)`)은 써도 됩니다.
- [ ] **Step 4: 테스트와 clippy가 통과해야 합니다.**
- [ ] **Step 5: `git commit -m "feat: mixer profile factory with 32 analog mixers"`**

### Task 6: 추천 엔진

**Files:** Create `src/advice.rs`. Modify `src/lib.rs`.

**Interfaces:**
- Consumes:
  - `mixers::{Control, ChannelGroup, KNOB_TRAVEL_DEG, EQ_RANGE_DB}`
  - `speech::Summary`
  - `target::{BANDS_HZ, ltass_db, Voice}`(테스트에서 씀)
- Produces:
  - `#[derive(Clone, Copy, Debug, PartialEq)] pub enum Shape { LowShelf(f32), HighShelf(f32), Bell(f32) }` + `impl Shape { pub fn response(&self, hz: f32) -> f32 }`. 식은 spec "추천 엔진"을 따르고, Bell은 σ = 0.6옥타브입니다.
  - `pub fn fit_eq(deviation: &[f32; 25], shapes: &[Shape]) -> (Vec<f32>, f32, f32)`
    - 반환값: (gains, offset c, 가중 잔차)
    - 사용 밴드와 가중치는 spec대로(100 Hz–12.5 kHz, 12.5k는 0.5), λ = 0.01
    - 정규방정식을 가우스 소거로 풉니다.
  - `#[derive(Clone, Debug, PartialEq)] pub enum Action { Wait, Ok, Rotate { degrees: f32, db: f32 }, SetFreq { hz: f32 }, SwitchOn }`. Rotate의 degrees는 양수면 시계방향입니다.
  - `#[derive(Clone, Debug)] pub struct Advice { pub label: String, pub control: Control, pub action: Action }`
  - `pub struct AdviceSet { pub knobs: Vec<Advice>, pub notes: Vec<String>, pub level_offset_db: f32 }`
  - `pub struct Inputs<'a> { pub summary: Option<&'a Summary>, pub target: &'a [f32; 25], pub clipped_recently: bool, pub feedback_hz: Option<f32>, pub silent: bool }`
  - `pub struct AdviceEngine`
    - `new() -> Self`
    - `advise(&mut self, group: &ChannelGroup, inp: &Inputs) -> AdviceSet`
  - `pub fn rotate_degrees(db: f32, deg_per_db: f32) -> f32`: 5° 단위로 반올림합니다.
  - `pub fn fmt_hz(hz: f32) -> String`: "400Hz", "2.5kHz", "12kHz", "1.25kHz"
- 라벨 규칙
  - `GAIN`, `저역컷 {fmt_hz}`, `HIGH {fmt_hz}`, `LOW {fmt_hz}`, `COMP`
  - 고정 MID는 `MID {fmt_hz}`
  - 스윕 MID 하나는 `FREQ`와 `MID` 두 항목을 이 순서로 냅니다. 스윕이 둘이면 `FREQ 1`/`MID 1`/`FREQ 2`/`MID 2`이고, 높은 대역이 1입니다.
- `advise` 동작(spec "추천 엔진"의 모든 규칙)
  - `silent`이거나 `summary`가 None이면 모든 노브가 `Wait`입니다. 문구는 각각 "입력 신호가 없습니다…"(macOS 권한 안내 포함)와 "말씀을 계속해 주세요"입니다.
  - 그 밖의 경우 아래 문구를 이 순서로 붙입니다: 하울링, 과부하, SNR.
  - 상태 초기화: `group.controls`가 지난 호출과 다르면 히스테리시스와 FREQ 상태를 초기화합니다. ✓ 상태의 초기값은 true입니다.
  - 순서와 충돌 규칙
    - COMP는 크레스트 규칙을 먼저 봅니다.
    - LOW 부스트는 최대 +3 dB입니다.
    - 저역컷은 c0 규칙을 따릅니다.
    - `level_offset_db`는 c입니다. UI가 목표 곡선을 그릴 때 씁니다.

- [ ] **Step 1: 테스트를 작성합니다.**

```rust
use crate::{mixers::*, speech::Summary, target::*};
fn summary(dev: impl Fn(f32) -> f32) -> Summary {
    let t = ltass_db(Voice::Male);
    let mut bands = [0.0; 25];
    for i in 0..25 { bands[i] = t[i] - 40.0 + dev(BANDS_HZ[i]); }
    Summary { bands_db: bands, peak_db: -12.0, rms_db: -30.0, noise_floor_db: -80.0, snr_db: 50.0,
              dynamic_range_db: 10.0, crest_db: 18.0, active_seconds: 4.0 }
}
fn run(e: &mut AdviceEngine, g: &ChannelGroup, s: &Summary) -> AdviceSet {
    let t = ltass_db(Voice::Male);
    e.advise(g, &Inputs { summary: Some(s), target: &t, clipped_recently: false, feedback_hz: None, silent: false })
}
fn act(a: &AdviceSet, label: &str) -> Action { a.knobs.iter().find(|k| k.label == label).unwrap().action.clone() }
fn db_of(a: Action) -> f32 { match a { Action::Rotate { db, .. } => db, x => panic!("{x:?}") } }
fn group(m: MixerModel, i: usize) -> ChannelGroup { MixerFactory::create(m).groups.remove(i) }
const V3: MixerModel = MixerModel::MackieProFx22v3;
const FX12: MixerModel = MixerModel::MackieProFx12v3;

#[test] fn flat_gives_all_ok() {
    let a = run(&mut AdviceEngine::new(), &group(V3, 0), &summary(|_| 0.0));
    assert_eq!(a.knobs.len(), 7);
    assert!(a.knobs.iter().all(|k| k.action == Action::Ok), "{:?}", a.knobs);
}
#[test] fn low_boom_turns_low_ccw() {
    let a = run(&mut AdviceEngine::new(), &group(V3, 0), &summary(|f| 6.0 * Shape::LowShelf(80.0).response(f)));
    let d = db_of(act(&a, "LOW 80Hz"));
    assert!((-7.5..=-3.0).contains(&d), "{d}");
    assert_eq!(act(&a, "HIGH 12kHz"), Action::Ok);
}
#[test] fn mid_bump_finds_freq() {
    let a = run(&mut AdviceEngine::new(), &group(V3, 0), &summary(|f| 6.0 * Shape::Bell(400.0).response(f)));
    match act(&a, "FREQ") { Action::SetFreq { hz } => assert!((315.0..=500.0).contains(&hz), "{hz}"), x => panic!("{x:?}") }
    assert!((-7.5..=-4.5).contains(&db_of(act(&a, "MID"))));
}
#[test] fn dull_top_turns_high_cw() {
    let a = run(&mut AdviceEngine::new(), &group(MixerModel::YamahaMgp24x, 0), &summary(|f| -6.0 * Shape::HighShelf(8_000.0).response(f)));
    assert!((4.0..=7.5).contains(&db_of(act(&a, "HIGH 8kHz"))));
}
#[test] fn rumble_recommends_lowcut() {
    let a = run(&mut AdviceEngine::new(), &group(V3, 0), &summary(|f| if f < 90.0 { 12.0 } else { 0.0 }));
    assert_eq!(act(&a, "저역컷 100Hz"), Action::SwitchOn);
    assert_eq!(act(&a, "LOW 80Hz"), Action::Ok);
}
#[test] fn clip_turns_gain_down() {
    let mut s = summary(|f| 20.0 * Shape::Bell(3_000.0).response(f));
    s.peak_db = -2.0;
    let t = ltass_db(Voice::Male);
    let a = AdviceEngine::new().advise(&group(V3, 0), &Inputs { summary: Some(&s), target: &t, clipped_recently: true, feedback_hz: None, silent: false });
    assert_eq!(act(&a, "GAIN"), Action::Rotate { degrees: -50.0, db: -10.0 });
    for k in &a.knobs { if let Action::Rotate { degrees, db } = k.action { assert!(degrees.is_finite() && db.is_finite() && db.abs() <= 20.0); } }
    for k in a.knobs.iter().filter(|k| k.label != "GAIN") { if let Action::Rotate { db, .. } = k.action { assert!(db.abs() <= 15.0); } }
}
#[test] fn quiet_turns_gain_up() {
    let mut s = summary(|_| 0.0);
    s.peak_db = -40.0;
    assert_eq!(act(&run(&mut AdviceEngine::new(), &group(V3, 0), &s), "GAIN"), Action::Rotate { degrees: 100.0, db: 20.0 });
    assert_eq!(act(&run(&mut AdviceEngine::new(), &group(MixerModel::YamahaMgp24x, 0), &s), "GAIN"), Action::Rotate { degrees: 135.0, db: 20.0 });
}
#[test] fn deadband_hysteresis() {
    let mut e = AdviceEngine::new();
    let g = group(FX12, 0);
    let boom = |x: f32| summary(move |f| x * Shape::LowShelf(80.0).response(f));
    assert_eq!(act(&run(&mut e, &g, &boom(2.2)), "LOW 80Hz"), Action::Ok);
    assert!(matches!(act(&run(&mut e, &g, &boom(5.0)), "LOW 80Hz"), Action::Rotate { .. }));
    assert!(matches!(act(&run(&mut e, &g, &boom(2.2)), "LOW 80Hz"), Action::Rotate { .. }));
    assert_eq!(act(&run(&mut e, &g, &boom(0.0)), "LOW 80Hz"), Action::Ok);
}
#[test] fn low_boost_capped() {
    let a = run(&mut AdviceEngine::new(), &group(FX12, 0), &summary(|f| -10.0 * Shape::LowShelf(80.0).response(f)));
    assert!(db_of(act(&a, "LOW 80Hz")) <= 3.0);
}
#[test] fn comp_rules() {
    let mut s = summary(|_| 0.0);
    s.dynamic_range_db = 22.0;
    assert_eq!(act(&run(&mut AdviceEngine::new(), &group(V3, 0), &s), "COMP"), Action::Rotate { degrees: 30.0, db: 0.0 });
    s.crest_db = 7.0; s.peak_db = -10.0;
    let a = run(&mut AdviceEngine::new(), &group(V3, 0), &s);
    assert_eq!(act(&a, "COMP"), Action::Rotate { degrees: -30.0, db: 0.0 });
    assert!(a.notes.iter().any(|n| n.contains("과부하")));
}
#[test] fn notes_for_feedback_and_snr() {
    let mut s = summary(|_| 0.0);
    s.snr_db = 20.0;
    let t = ltass_db(Voice::Male);
    let a = AdviceEngine::new().advise(&group(V3, 0), &Inputs { summary: Some(&s), target: &t, clipped_recently: false, feedback_hz: Some(2_500.0), silent: false });
    assert!(a.notes.iter().any(|n| n.contains("2.5kHz")));
    assert!(a.notes.iter().any(|n| n.contains("마이크를")));
}
#[test] fn silent_input_waits() {
    let t = ltass_db(Voice::Male);
    let mut e = AdviceEngine::new();
    let a = e.advise(&group(V3, 0), &Inputs { summary: None, target: &t, clipped_recently: false, feedback_hz: None, silent: true });
    assert!(a.knobs.iter().all(|k| k.action == Action::Wait));
    assert!(a.notes.iter().any(|n| n.contains("입력 신호가 없습니다")));
    let b = e.advise(&group(V3, 0), &Inputs { summary: None, target: &t, clipped_recently: false, feedback_hz: None, silent: false });
    assert!(b.notes.iter().any(|n| n.contains("말씀을 계속해 주세요")));
}
#[test] fn group_change_resets_state() {
    let mut e = AdviceEngine::new();
    assert_eq!(run(&mut e, &group(V3, 0), &summary(|_| 0.0)).knobs.len(), 7);
    assert_eq!(run(&mut e, &group(MixerModel::YamahaMg06x, 0), &summary(|_| 0.0)).knobs.len(), 4);
}
#[test] fn rounding_and_labels() {
    assert_eq!(rotate_degrees(1.6, 10.0), 15.0);
    assert_eq!(rotate_degrees(-3.2, 10.0), -30.0);
    assert_eq!(rotate_degrees(2.4, 10.0), 25.0);
    assert_eq!([fmt_hz(400.0), fmt_hz(2_500.0), fmt_hz(12_000.0), fmt_hz(1_250.0), fmt_hz(80.0)],
               ["400Hz", "2.5kHz", "12kHz", "1.25kHz", "80Hz"]);
}
```
- [ ] **Step 2: 컴파일 실패를 확인합니다.**
- [ ] **Step 3: 구현합니다.**
  - 편차 = `summary.bands_db − target`
  - 스윕 MID의 후보는 범위 안의 `BANDS_HZ`입니다. 스윕이 여럿이면 좌표 하강을 2회 돌립니다. 이전 주파수의 잔차보다 20% 이상 좋을 때만 바꿉니다.
  - 노브별 deg/dB: EQ는 `KNOB_TRAVEL_DEG/(2·EQ_RANGE_DB)`, GAIN은 `KNOB_TRAVEL_DEG/range_db`
  - COMP 회전은 `db: 0.0`, ±30°로 고정합니다. `// ponytail:` 주석에 휴리스틱의 한계를 적습니다.
- [ ] **Step 4: 테스트와 clippy가 통과해야 합니다.** 허용 범위를 넓히려면 엔진 쪽 원인을 먼저 고쳐야 합니다. 테스트 범위를 느슨하게 하는 것은 리뷰어 승인이 있을 때만 합니다.
- [ ] **Step 5: `git commit -m "feat: knob recommendation engine"`**

### Task 7: 오디오 입력, 테마, 첫 실행 화면

**Files:**
- Create: `src/audio.rs`, `src/theme.rs`, `src/app.rs`
- Modify: `src/lib.rs`(`pub mod audio; pub mod theme;`), `src/main.rs`(전체 교체)

**Interfaces:**
- Produces (`audio.rs`):
  - `pub struct Input { pub sample_rate: f32, pub channels: usize, pub rx: rtrb::Consumer<f32>, _keep: Box<dyn std::any::Any> }`. `_keep`은 cpal::Stream이나 데모 스레드 가드입니다.
  - `pub fn device_names() -> Vec<String>`
  - `pub fn open(device: Option<&str>) -> Result<Input, String>`
    - 이름이 같은 입력 장치를 찾고, 없으면 기본 장치를 씁니다.
    - `default_input_config`의 `sample_format()`을 match해서 F32/I16/I32/U16 등을 `to_sample::<f32>()`로 변환합니다.
    - 링 용량은 sr×ch×2입니다. push가 실패하면 버립니다.
    - `stream.play()`를 호출합니다.
  - `pub fn open_demo() -> Input`
    - 48 kHz, 1채널입니다. 스레드가 10 ms마다 합성 음성을 넣습니다. 합성 음성은 T4 `voiced`와 같은 식에 0.6 s 켜짐 / 0.25 s 꺼짐 포락선, 120 Hz 붐 +6 dB를 더한 것입니다.
    - `Producer::is_abandoned()`이면 스레드를 끝냅니다.
  - `pub fn pick_channel(interleaved: &[f32], channels: usize, ch: usize) -> Vec<f32>`: `ch >= channels`면 0번 채널을 씁니다.
- Produces (`theme.rs`):
  - `pub mod mocha { pub const BASE: Color32 = …; … }`: spec의 17색이고, 파일 머리에 출처와 MIT 표기를 남깁니다.
  - `pub fn install(ctx: &egui::Context)`
    - Pretendard를 `include_bytes!("../assets/Pretendard-Regular.otf")`로 Proportional 0번에 넣고, Monospace 뒤에도 붙입니다.
    - dark Visuals를 mocha로 매핑합니다: panel/window는 base/mantle, extreme_bg는 crust, widgets는 surface0/1/2, selection은 blue, 글자는 text.
- Produces (`app.rs`, 바이너리 전용):
  - `pub struct App`
  - `App::new(cc: &eframe::CreationContext, demo: bool) -> Self`
  - `impl eframe::App for App { fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) }`. eframe 0.37의 시그니처입니다.
  - 매 프레임 할 일
    1. `rx`를 비웁니다.
    2. LevelMeter에 원본을 넣습니다.
    3. `pick_channel`로 Spectrum에 넣습니다.
    4. 프레임마다 third_octave → SpeechStats, FeedbackDetector, display_db → DisplayCurve를 처리합니다.
    5. `request_repaint()`
- `main.rs`
  - `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`
  - `--demo` 인자를 받습니다.
  - viewport는 1100×760이고 최소 900×640입니다.
  - 제목은 "믹서 분석기"입니다.
- 이 Task의 화면 범위
  - 상단 바: 입력장치 콤보(+ "데모 신호"), 분석채널 콤보
  - 스펙트럼 플롯: x = log10(Hz), 20 Hz–20 kHz, 눈금 라벨 20/50/100/200/500/1k/2k/5k/10k/20k; y는 −90…0. 선은 현재(blue)와 피크 홀드(peach)입니다.
  - 장치를 바꾸면 `Input`과 분석 상태를 새로 만듭니다. 열기에 실패하면 문구를 띄웁니다.

- [ ] **Step 1: `audio.rs` 테스트를 작성합니다.**

```rust
#[test] fn pick_channel_falls_back_to_first() {
    assert_eq!(pick_channel(&[1.0, 2.0, 3.0, 4.0], 2, 1), vec![2.0, 4.0]);
    assert_eq!(pick_channel(&[1.0, 2.0, 3.0, 4.0], 2, 0), vec![1.0, 3.0]);
    assert_eq!(pick_channel(&[1.0, 2.0], 1, 1), vec![1.0, 2.0]);
}
#[test] fn demo_input_streams_samples() {
    let inp = open_demo();
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert_eq!((inp.sample_rate, inp.channels), (48_000.0, 1));
    assert!(inp.rx.slots() > 1000);
}
```
- [ ] **Step 2: 실패를 확인합니다.**
- [ ] **Step 3: `audio.rs`, `theme.rs`, `app.rs`, `main.rs`를 구현합니다.**
- [ ] **Step 4: `cargo test`와 clippy가 통과해야 합니다. 이어서 실행 스모크를 합니다.**
  - 명령: `perl -e 'alarm 8; exec @ARGV' cargo run -- --demo 2>run.log; grep -c panicked run.log`
  - 기대: 알람으로 종료(exit 142)되고 grep 결과가 0입니다.
  - `run.log`는 커밋하지 않습니다.
- [ ] **Step 5: `git commit -m "feat: audio input, Catppuccin theme, live spectrum window"`**

### Task 8: 전체 화면(기록, 상태, 노브 추천, 안내)

**Files:** Modify `src/app.rs`.

**Interfaces:**
- Consumes: T2–T6 전부. `MixerFactory`, `MixerModel::ALL`, `AdviceEngine`, `fmt_hz`, `theme::mocha`
- Produces: `pub fn dial_angle(hz: f32, lo: f32, hi: f32) -> f32`. 12시 기준 각도(°)이고 lo는 −150, hi는 +150이며 로그 선형입니다.
- 상단 바에 추가하는 것
  - 믹서 콤보: `"{brand} {name}"`, 기본값 `MackieProFx22v3`
  - 채널 그룹 콤보
  - 목표 음성 토글: 남성 / 여성
  - [클리핑 리셋] 버튼
- 스펙트럼에 목표 곡선을 겹칩니다: `ltass + level_offset_db − 3.01`을 1/3옥타브 점에 점선(subtext0)으로 그립니다.
- 레벨 기록 플롯
  - 최근 30초 피크 선입니다. 색은 −12 dB 이하 green, −6 dB까지 yellow, 그 위 red입니다.
  - 클리핑 점은 red 마커로 표시합니다.
  - 아래 상태 줄: `피크 {:.0} dBFS · 클리핑 {n}회 · 과부하 의심 ●/○ · 하울링 ●/○ {hz}`
- 노브 줄: 추천을 0.5 s마다 다시 계산합니다. 입력은 `clipped_within(4.0)`, `silent_for(3.0)`, 피드백 결과입니다.
  - 회전 노브: 원, 12시 포인터, 추천 각도만큼의 호와 화살촉(yellow)을 그립니다. 글자는 `시계방향 30°` 또는 `반시계 15°`와 `(≈+3dB)`입니다.
  - Ok: green "✓"
  - Wait: subtext0 "—"
  - 스위치: 사각형을 그리고, `켜기 권장`(sapphire) 또는 `✓`
  - FREQ: 다이얼에 `dial_angle`로 절대 위치를 그리고, 글자는 `{fmt_hz} 눈금에`
  - GAIN의 dB 표기는 `(≈-10dB)`입니다.
- 하단 안내: `AdviceSet.notes`를 차례로 쓰고, 그 밑에 `MixerProfile.pc_tip`을 씁니다.
- 믹서나 그룹을 바꾸면 `AdviceEngine::new()`로 교체합니다.

- [ ] **Step 1: `app.rs`에 테스트를 작성합니다.**

```rust
#[test] fn dial_angle_is_log_linear() {
    assert!((dial_angle(100.0, 100.0, 8_000.0) + 150.0).abs() < 1e-3);
    assert!((dial_angle(8_000.0, 100.0, 8_000.0) - 150.0).abs() < 1e-3);
    assert!(dial_angle((100.0f32 * 8_000.0).sqrt(), 100.0, 8_000.0).abs() < 1e-2);
}
```
- [ ] **Step 2: 실패를 확인합니다.**
- [ ] **Step 3: 구현합니다.**
- [ ] **Step 4: 테스트, clippy, T7의 실행 스모크가 모두 통과해야 합니다.**
- [ ] **Step 5: `git commit -m "feat: level history, knob advice row and tips"`**

### Task 9: 패키징과 README

**Files:** Create `scripts/build-mac.sh`, `scripts/build-win.sh`, `packaging/Info.plist`, `README.md`.

**Interfaces:**
- Consumes: 바이너리 이름 `mixer-analyzer`
- Produces: `dist/MixerAnalyzer-mac.zip`(MixerAnalyzer.app + README.md), `dist/MixerAnalyzer-win.zip`(MixerAnalyzer.exe + README.md)
- `Info.plist`
  - CFBundleName "믹서 분석기", CFBundleExecutable `mixer-analyzer`, CFBundleIdentifier `org.church.mixer-analyzer`
  - LSMinimumSystemVersion 11.0, NSHighResolutionCapable true
  - NSMicrophoneUsageDescription "믹서에서 들어오는 소리를 분석하려면 오디오 입력 권한이 필요합니다."
- `build-mac.sh` 순서
  1. `cargo build --release --target aarch64-apple-darwin`과 `--target x86_64-apple-darwin`
  2. `lipo -create`
  3. `.app/Contents/{MacOS,Info.plist}`를 만듭니다.
  4. `codesign -s - -f --deep`
  5. `ditto -c -k --keepParent`로 압축합니다.
- `build-win.sh`: `PATH=/opt/homebrew/opt/llvm/bin:$PATH cargo xwin build --release --target x86_64-pc-windows-msvc`를 돌린 뒤 exe 이름을 `MixerAnalyzer.exe`로 바꾸고 zip으로 묶습니다.
- `README.md`(한국어)
  - 설치
  - 첫 실행: mac은 시스템 설정 → 개인정보 보호 및 보안 → "확인 없이 열기"와 마이크 권한, Windows는 SmartScreen "추가 정보 → 실행"
  - 믹서별 연결: spec 팁을 요약합니다.
  - 화면 읽는 법
  - 라이선스 고지: Pretendard OFL(`assets/Pretendard-LICENSE.txt`도 zip에 넣음), Catppuccin MIT

- [ ] **Step 1: 스크립트, plist, README를 작성합니다.**
- [ ] **Step 2: `bash scripts/build-mac.sh && bash scripts/build-win.sh`를 실행합니다.**
- [ ] **Step 3: 검증합니다. 모두 기대대로 나와야 합니다.**
  - `lipo -archs dist/MixerAnalyzer.app/Contents/MacOS/mixer-analyzer` → `x86_64 arm64`
  - `codesign -dv dist/MixerAnalyzer.app 2>&1 | grep -c adhoc` → `1`
  - `plutil -p dist/MixerAnalyzer.app/Contents/Info.plist | grep -c NSMicrophoneUsageDescription` → `1`
  - `file dist/MixerAnalyzer.exe` → `PE32+ … x86-64`
  - `/opt/homebrew/opt/llvm/bin/llvm-objdump -p dist/MixerAnalyzer.exe | grep -ci vcruntime` → `0`
  - `unzip -l` 결과로 두 zip에 README.md와 라이선스 파일이 있는지 봅니다.
- [ ] **Step 4: `git add scripts packaging README.md && git commit -m "build: mac universal app and windows exe packaging"`**(dist/는 커밋하지 않습니다.)

---
업로드(Claude in Chrome → Drive 폴더 "mixer-analyzer")는 코딩 작업이 아니라서 컨트롤러가 직접 합니다.
