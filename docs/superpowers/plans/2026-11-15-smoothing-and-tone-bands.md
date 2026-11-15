# 주파수 평활, 대역 비교, 봉우리 표시 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 그래프에 1/N옥타브 주파수 평활(기본 1/6), 기준 대비 이름 붙은 8개 구간의 에너지 비교 띠, 좁게 튀는 봉우리 표시를 넣는다.

**Architecture:** `src/spectrum.rs::display_db`에 평활 인자를 더하고(0 = 지금 방식) `peaks`를 추가한다. `src/target.rs`에 구간 표 `TONE_BANDS`와 `tone_excess`를 추가한다. `src/app.rs`는 콤보·체크박스와 그리기만 맡는다.

**Tech Stack:** Rust 1.96, eframe/egui 0.37, egui_plot 0.38.

**Spec:** `docs/superpowers/specs/2026-11-15-smoothing-and-tone-bands-design.md`

## Global Constraints

- 새 의존성 없음. 추천 엔진·하울링 감지는 바꾸지 않는다.
- 평활 콤보 이름 `주파수 평활`, 항목 `없음` / `1/24옥타브` / `1/12옥타브` / `1/6옥타브` / `1/3옥타브` (N = 0, 24, 12, 6, 3), 기본 `1/6옥타브`. `없음`(N=0)은 지금 `display_db` 결과와 비트 단위로 같다.
- 체크박스 `대역 비교`(이름 색 `mocha::RED`, 기본 켜짐), `봉우리 표시`(이름 색 `mocha::PEACH`, 기본 꺼짐).
- 구간(이름, lo, hi Hz): 웅웅거림 80–250, 먹먹함 250–500, 통 울림 500–800, 비음 800–1500, 까랑함 1500–3000, 쏘는 소리 3000–5000, 치찰음 5000–10000, 공기감 10000–20000. 1/3옥타브 대역 중심이 [lo, hi)에 들면 그 구간.
- 대역 비교값 = 10·log10(Σ 측정 파워 / Σ 기준 파워), 기준 = `goal.db + level_offset_db`, 측정 = `Summary.bands_db`. ±3 dB 이상만 칠함(많으면 RED 반투명, 적으면 BLUE 더 옅은 반투명). 라벨 `{이름}`과 `{:+.1} dB`.
- 봉우리: 1/6옥타브 평활 곡선 − 1옥타브 평활 곡선 ≥ 3 dB, ±1/6옥타브(해상도 점 수 per_oct/6) 안 최대, 100 Hz ≤ f ≤ 10 kHz, 큰 순 최대 5개. 라벨 `892Hz +4dB`(1 kHz 미만 정수 Hz), `2.5kHz +4dB`(1 kHz 이상 소수 한 자리 kHz), dB는 정수 반올림.
- 대역 비교·봉우리는 분석 결과(`Summary`)가 있을 때만(대역 비교는 추천도 있을 때만).
- 커밋 메시지는 제목 한 줄만(본문·Co-Authored-By 없음).

## Review Focus

- 분석 중이거나 실행 중에 평활·해상도를 바꿔도 곡선 길이가 맞아 패닉이 없어야 한다. (Task 1 앱 확인: 평활 변경 시 `DisplayCurve`를 새로 만듦)
- 44.1 kHz 장치에서도 평활이 같은 모양이어야 한다(칸 폭이 표본율에서 나옴). (Task 1 `smoothing_keeps_flat_level`을 44 100에서도)
- 평평한 소리에서 평활해도 레벨이 그대로여야 목표선 위치가 맞는다. (Task 1 `smoothing_keeps_flat_level`)
- 말소리가 없던 분석(`summary` 없음)이나 추천 없음 → 띠·봉우리 없이 정상. (Task 2·3 앱 확인)
- 넓게 올라간 대역은 봉우리로 잡히지 않는다(대역 비교 몫). (Task 3 `broad_rise_is_not_a_peak`)

---

### Task 1: 주파수 평활

**Files:**
- Modify: `src/spectrum.rs` (`display_db`와 그 테스트 호출), `src/app.rs` (콤보, `Live::pump`, `result_db`, `Live::new`가 곡선을 만드는 곳), `README.md`

**Interfaces:**
- Produces: `pub fn display_db(bin_power: &[f32], sample_rate: f32, per_oct: usize, smooth_n: usize) -> Vec<f32>` (`smooth_n` 0 = 지금 방식)

- [ ] **Step 1: 실패하는 테스트** (`src/spectrum.rs` tests; 기존 `display_db(..)` 호출에는 `, 0`을 붙인다)

```rust
/// Flat power keeps the display scale: mean power × the display band's (fractional, ≥ 1) bin count.
/// (Not compared to `smooth_n = 0`: that one counts whole bins, which is the sawtooth being removed.)
#[test]
fn smoothing_keeps_flat_level() {
    let h = 2f32.powf(0.5 / 48.0);
    for sr in [48_000.0f32, 44_100.0] {
        let flat = vec![1e-6f32; FFT_SIZE / 2 + 1];
        let bw = sr / FFT_SIZE as f32;
        for n in [24, 12, 6, 3] {
            for (f, d) in display_freqs(48).iter().zip(display_db(&flat, sr, 48, n)) {
                let want = db(1e-6 * (f * (h - 1.0 / h) / bw).max(1.0));
                assert!((d - want).abs() < 0.01, "sr {sr} n {n} f {f}: {d} vs {want}");
            }
        }
    }
}

#[test]
fn smoothing_removes_low_steps() {
    let sr = 48_000.0;
    let rising: Vec<f32> = (0..=FFT_SIZE / 2).map(|k| 1e-6 * (1.0 + k as f32)).collect();
    let fs = display_freqs(48);
    let low: Vec<usize> = (0..fs.len()).filter(|&i| fs[i] > 40.0 && fs[i] < 150.0).collect();
    let none = display_db(&rising, sr, 48, 0);
    assert!(low.windows(2).any(|w| none[w[0]] == none[w[1]]), "old method has steps");
    let six = display_db(&rising, sr, 48, 6);
    assert!(low.windows(2).all(|w| six[w[1]] > six[w[0]]), "smoothed rises every point");
}
```

- [ ] **Step 2:** `cargo test --lib spectrum` → 컴파일 실패(인자 개수).
- [ ] **Step 3: 구현** — 스펙 "접근 방식 · 평활" 공식 그대로. `smooth_n == 0`이면 지금 본문 그대로.
- [ ] **Step 4: 앱** — `const FREQ_SMOOTH: [(&str, usize); 5]`(항목 Global Constraints), `App`에 인덱스 필드(기본 3 = `1/6옥타브`), `해상도` 콤보 바로 아래 `주파수 평활` 콤보(같은 모양). `Live::pump`와 `result_db`가 선택한 N을 `display_db`에 넘긴다. 평활을 바꾸면 해상도 변경 때처럼 `live.curve`를 새 `DisplayCurve`로 바꾼다.
- [ ] **Step 5: README** — "화면 읽는 법"의 해상도 설명 다음에 한 항목: 주파수 평활은 선을 주변 1/N옥타브 평균으로 매끄럽게 하고(기본 1/6), 세게 할수록 좁은 봉우리가 낮게 보이며, 노브 추천에는 영향이 없다.
- [ ] **Step 6: 확인** — `cargo test` 전부 통과, `cargo clippy --all-targets -- -D warnings` 경고 없음.
- [ ] **Step 7: 커밋** — `git commit -m "feat: fractional-octave smoothing for the spectrum graph"`

### Task 2: 대역 비교

**Files:**
- Modify: `src/target.rs` (표·함수·테스트), `src/app.rs` (`Show`, 체크박스, 그리기), `README.md`

**Interfaces:**
- Consumes: Task 1의 `display_db` 4인자(그리기에는 안 씀).
- Produces:
  - `pub const TONE_BANDS: [(&str, f32, f32); 8]` (이름, lo, hi — Global Constraints 순서·값)
  - `pub fn tone_excess(measured: &[f32; 25], reference: &[f32; 25]) -> [f32; 8]`

- [ ] **Step 1: 실패하는 테스트** (`src/target.rs` tests)

```rust
#[test]
fn tone_bands_partition_third_octaves() {
    for (i, &hz) in BANDS_HZ.iter().enumerate() {
        let n = TONE_BANDS.iter().filter(|&&(_, lo, hi)| (lo..hi).contains(&hz)).count();
        assert_eq!(n, if i == 0 { 0 } else { 1 }, "{hz} Hz");
    }
}

#[test]
fn tone_excess_sums_power() {
    let reference = [0.0f32; 25];
    assert!(tone_excess(&reference, &reference).iter().all(|d| d.abs() < 1e-4));
    let mut m = reference;
    m[15] = 6.0; // 2 kHz only: 10·log10((1 + 10^0.6 + 1) / 3) ≈ 3.0, not the dB mean 2.0
    let e = tone_excess(&m, &reference);
    assert!((e[4] - 2.996).abs() < 0.01, "{}", e[4]);
    assert!(e.iter().enumerate().all(|(i, d)| i == 4 || d.abs() < 1e-4));
}

#[test]
fn tone_excess_follows_reference_level() {
    let reference = [-40.0f32; 25];
    let mut m = reference;
    for i in 14..=16 { m[i] += 6.0; } // 1600, 2000, 2500 = 까랑함
    let e = tone_excess(&m, &reference);
    assert_eq!(TONE_BANDS[4].0, "까랑함");
    assert!((e[4] - 6.0).abs() < 0.01 && e[3].abs() < 1e-4 && e[5].abs() < 1e-4);
}
```

- [ ] **Step 2:** `cargo test --lib target` → 컴파일 실패.
- [ ] **Step 3: 구현** `TONE_BANDS`, `tone_excess`.
- [ ] **Step 4: 앱** — `Show`에 `tone: bool`(기본 true), `표시` 줄의 `노브 구간` 뒤에 `대역 비교` 체크박스. 분석 결과(`summary`)와 추천(`advice`)이 있고 켜져 있을 때, 기준 = `self.goal.db[i] + advice.level_offset_db`로 `tone_excess(&summary.bands_db, &기준)`을 구해 `spectrum_plot`에 넘긴다. 그래프에서 각 구간 x = log10(lo)…log10(hi)에 세로 띠(값 ≥ 3: RED 반투명, ≤ −3: BLUE 더 옅은 반투명, 그 사이 없음), 그래프 아래쪽 안(노브 막대와 겹치지 않는 높이)에 이름과 `{:+.1} dB`(±3 이상은 그 색, 나머지 `mocha::SUBTEXT0`). 띠는 곡선 뒤에 그린다.
- [ ] **Step 5: README** — "화면 읽는 법"에 대역 비교 항목: 분석 결과를 목표(또는 저장한 소리)와 8개 구간(이름과 범위 나열)으로 비교해 3 dB 이상 많으면 빨강, 적으면 파랑; 2.4–3.6 kHz처럼 좋은 울림도 있는 대역이라 목표 대비로 본다; 치찰음처럼 순간적인 소리는 평균에 덜 보인다.
- [ ] **Step 6: 확인** — `cargo test`, `cargo clippy --all-targets -- -D warnings`.
- [ ] **Step 7: 커밋** — `git commit -m "feat: tone band comparison against the target"`

### Task 3: 봉우리 표시

**Files:**
- Modify: `src/spectrum.rs` (함수·테스트), `src/app.rs` (`Show`, 체크박스, 그리기), `README.md`

**Interfaces:**
- Consumes: Task 1 `display_db(.., smooth_n)`, `display_freqs`.
- Produces: `pub fn peaks(bin_power: &[f32], sample_rate: f32, per_oct: usize) -> Vec<(f32, f32)>` — (Hz, 주변보다 높은 dB), 큰 순, 최대 5개.

- [ ] **Step 1: 실패하는 테스트** (`src/spectrum.rs` tests)

```rust
fn shaped(gain_db: impl Fn(f32) -> f32) -> Vec<f32> {
    let bw = 48_000.0 / FFT_SIZE as f32;
    (0..=FFT_SIZE / 2).map(|k| 1e-6 * 10f32.powf(gain_db((k as f32 * bw).max(1.0)) / 10.0)).collect()
}

#[test]
fn flat_has_no_peaks() {
    assert!(peaks(&shaped(|_| 0.0), 48_000.0, 48).is_empty());
}

#[test]
fn narrow_bump_is_a_peak() {
    let p = peaks(&shaped(|f| 10.0 * (-(f / 1000.0).log2().powi(2) / (2.0 * 0.04f32.powi(2))).exp()), 48_000.0, 48);
    assert_eq!(p.len(), 1, "{p:?}");
    assert!((p[0].0 / 1000.0).log2().abs() < 1.0 / 12.0 && p[0].1 >= 3.0, "{p:?}");
}

#[test]
fn broad_rise_is_not_a_peak() {
    let p = peaks(&shaped(|f| 8.0 * (-(f / 3200.0).log2().powi(2) / (2.0 * 0.5f32.powi(2))).exp()), 48_000.0, 48);
    assert!(p.is_empty(), "{p:?}");
}

#[test]
fn at_most_five_largest_first() {
    let centres = [150.0f32, 300.0, 600.0, 1200.0, 2400.0, 4800.0, 9000.0];
    let p = peaks(&shaped(|f| centres.iter().enumerate()
        .map(|(i, c)| (4.0 + i as f32) * (-(f / c).log2().powi(2) / (2.0 * 0.04f32.powi(2))).exp())
        .sum()), 48_000.0, 48);
    assert_eq!(p.len(), 5, "{p:?}");
    assert!(p.windows(2).all(|w| w[0].1 >= w[1].1), "{p:?}");
}
```

- [ ] **Step 2:** `cargo test --lib spectrum` → 컴파일 실패.
- [ ] **Step 3: 구현** `peaks` — `display_db(bin_power, sr, per_oct, 6)` − `display_db(.., 1)`, 스펙·Global Constraints 규칙.
- [ ] **Step 4: 앱** — `Show`에 `bumps: bool`(기본 false), `대역 비교` 뒤에 `봉우리 표시` 체크박스. 분석 결과가 있고 켜져 있으면 `peaks(&summary.bin_power, sample_rate, per_oct)`의 각 Hz에서, 화면의 분석 결과 곡선(선택한 평활) 가장 가까운 점 위에 PEACH ▼(egui_plot `Points` + `MarkerShape::Down`)와 그 위 라벨(Global Constraints 형식).
- [ ] **Step 5: README** — 봉우리 표시 항목: 주변 1옥타브 평균보다 3 dB 이상 좁게 튀는 곳 최대 5개; 원래 목소리의 울림(포먼트)에도 붙으니 대역 비교와 함께 보고, 넓게 올라간 대역은 잡지 않으며, 하울링은 하울링 표시로 본다.
- [ ] **Step 6: 확인** — `cargo test`, `cargo clippy --all-targets -- -D warnings`.
- [ ] **Step 7: 커밋** — `git commit -m "feat: bump markers on the analysis curve"`
