# 그대로 분석에서 노브 위치 입력 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 그대로 분석 뒤 사용자가 노브 위치를 입력하고, 엔진은 그 위치에서의 추천(상대 조정)을 주며, 저장 세팅은 소리와 입력한 노브 위치(라벨)를 갖는다. 0.4.0의 노브 위치 추정은 뺀다.

**Architecture:** `advice.rs`에 `Positions`(라벨→값)와 도우미를 두고, 엔진 `Inputs.current`로 지금 위치를 받아 "측정 − K의 EQ 모양 = 가운데 소리"로 계산하며 ✓·FREQ·저역컷·COMP를 K 기준으로 판단한다. `saved.rs`는 `Flat` 대신 31번째 필드에 노브 위치를 쓴다. `app.rs`는 입력 칸, 노브 줄 규칙 1–6, 저장 규칙, 다이얼·설명을 순수 함수 중심으로 바꾼다.

**Tech Stack:** Rust 1.96, eframe/egui 0.37.

**Spec:** `docs/superpowers/specs/2026-11-16-entered-knobs-design.md` (바꾸는 대상: `2026-11-16-knob-settings-design.md`의 추정 부분, 바탕: `2026-11-16-flat-start-design.md`)

## Global Constraints
- spec의 한국어 문구(머리글, 버튼, 노트, 설명 형식, 안내창 흐린 글씨)는 글자 그대로.
- 빌드·테스트는 `CARGO_TARGET_DIR=/private/tmp/claude-501/-Users-sli/6b230aad-5cf4-4c59-a8ab-cfdc1c61b35f/scratchpad/vis-target`. `cargo test -q`와 `cargo clippy -q --all-targets`가 깨끗해야 한다(시작: lib 112 + bin 31 통과, 1 ignored).
- 커밋은 제목 한 줄(본문·트레일러 없음), conventional prefix. 저장소 루트의 스크린샷 PNG 두 개는 커밋하지 않는다.
- 브랜치 `feat/entered-knobs`(main 894958d 위).
- `current: None`(가운데)일 때 엔진 결과는 지금과 같아야 한다(기존 엔진 테스트가 그대로 통과).

## Review Focus
1. 라벨대로 맞춘 같은 소리를 그대로 분석 + 그 라벨 입력 → 모든 노브 ✓. 하나라도 돌리라고 하면 봉사자가 멀쩡한 세팅을 망친다(Task 1, 모든 믹서·그룹).
2. 다른 그룹에서 입력한 위치나 다른 믹서·그룹의 저장 위치는 쓰지 않는다(`fits`와 믹서·그룹 일치, Task 3).
3. 확인 전 그대로 분석: 추천도 저장도 없다(Task 3).
4. 0.4.0의 56필드 줄은 읽지 못한 줄이 아니라 소리만 읽힌다. 아니면 0.4.0에서 저장한 사람의 파일이 쓰기 막힘(Task 2).
5. 미리 채움은 분석 직전 노브 줄이 보여 주던 목표 위치다(분석 뒤 다시 계산한 추천이 아님, Task 3).

---

### Task 1: 엔진이 지금 노브 위치를 받는다

**Files:** Modify `src/advice.rs` (types, helpers, `Inputs`, `AdviceEngine::advise`, tests). Every `Inputs { .. }` literal in `src/app.rs` and tests gets `current: None`.

**Interfaces (Produces):**
- `pub type Positions = Vec<(String, f32)>;` — `labels(group)`의 라벨(GAIN 제외) 순서대로. 값: EQ dB(가운데에서), FREQ Hz(0 = 모름), 저역컷 1/0, COMP 왼쪽 끝에서 °.
- `pub fn centre(group: &ChannelGroup) -> Positions` — 모두 0.
- `pub fn fits(group: &ChannelGroup, p: &Positions) -> bool` — 라벨이 같은 순서로 정확히 같음.
- `pub fn freq_choices(control: Control) -> Vec<f32>` — 스윕 MID의 FREQ 후보(엔진의 `cands`와 같은 것; 엔진도 이것을 쓴다). 스윕이 아니면 빈 목록.
- `pub fn targets(knobs: &[Advice], from: &Positions) -> Positions` — 추천을 따른 뒤의 위치: EQ `Rotate` → `db`, COMP `Rotate` → from + degrees, `SetFreq` → hz, `SwitchOn` → 1, 그 밖(✓·Wait) → from의 값. GAIN은 건너뜀.
- `pub fn as_advice(group: &ChannelGroup, p: &Positions) -> Vec<Advice>` — 저장된 위치를 가운데에서 그릴 동작으로: EQ ≠ 0 → `Rotate { degrees: rotate_degrees(db, 10°/dB), db }`, FREQ(그 MID ≠ 0이고 값 > 0) → `SetFreq`, 저역컷 1 → `SwitchOn`, COMP > 0 → `Rotate { degrees: K, db: 0.0 }`, 나머지와 GAIN → `Ok`.
- `Inputs.current: Option<&'a Positions>` — `None` 또는 `fits`가 아니면 가운데. 동작은 spec "엔진" 절 그대로:
  - 측정 대역 − K의 EQ 모양(Shape 응답 × dB, 저역컷 켬이면 + `hpf_db` 빼기)으로 가운데 소리를 만들어 지금 계산을 돌린다. 한도는 목표 위치에 건다. 대역 가드는 측정한 소리(`tone_excess(s.bands_db, ..)`)로 판단해 K에서 건다(빨강: 목표 ≤ K, 파랑: 목표 ≥ K).
  - EQ ✓ = |목표 − K| ≤ 그 노브의 ✓ 범위(새 엔진). `Rotate { degrees: rotate_degrees(목표 − K, ..), db: 목표 }`. ✓ 범위는 복구 모드이고 `current`가 그룹에 맞으면 max(`OK_RELEASE_DB`, `NOISE_Z` · `TAKE_NOISE_DB` · √diag `gain_cov`), 아니면 `OK_RELEASE_DB`. `gain_cov(shapes)`는 대역마다 독립인 1 dB 잡음에서 맞춘 게인의 공분산(M⁻¹AᵀW²AM⁻¹, 맞춤이 d에 선형이라 대역마다 1 dB만 넣은 `fit_eq_in`의 게인끼리 곱한 합), 고른 주파수에서. 다만 제 범위 안에 남은 노브 q만의 돌릴 양 t_q로 d² = t_qᵀ`gain_cov`_qq⁻¹t_q / `TAKE_NOISE_DB`²가 `CHI2_999`[q 수]를 넘으면 모두 `OK_RELEASE_DB`(`solve`는 `fit_eq_in`의 가우스 소거를 뺀 것). `TAKE_NOISE_DB` 0.8·`NOISE_Z` 2.75는 균일 ±1 dB 잡음 시뮬레이션으로 고른 값(ponytail 주석).
  - K의 FREQ(> 0)를 `self.freq`에 넣고 시작(히스테리시스). FREQ는 고른 hz ≠ K의 FREQ이고 |MID 목표| > 그 MID의 ✓ 범위면 `SetFreq`, 아니면 ✓(K의 MID가 0이면 MID의 ✓ 상태).
  - K 저역컷 켬: ✓, 맞춤은 켜진 것으로. 끔: 지금처럼 켜기 판단.
  - COMP: K ≥ 30 → ✓; 아니면 지금 규칙이 참일 때 `Rotate { degrees: 30 − K, db: 0.0 }`.
- `AdviceSet.level_offset_db` 뜻은 그대로(측정 ≈ 목표 + offset).

- [ ] **Step 1: Write the failing tests** (advice.rs tests; build the finished/measured sound with the engine's own `Shape::response` and `hpf_db`, raw = `summary(|_| 0.0)`'s bands):
  - `same_sound_with_entered_positions_is_all_ok`: for every `MixerModel` × every group: K = EQ values cycling `[6, -6, -3, 9, 4]` in EQ order, first sweep FREQ = its highest candidate and a second sweep its lowest, 저역컷 1 if present, COMP 30 if present. measured = target = raw + model(K), restore DR = measured DR, `current: Some(&K)` → every knob except GAIN is `Ok`.
  - `entered_matches_centre_on_fixed_mid`: ProFX12v3 group 0 (fixed MID). raw = male LTASS − 40 − EQ(HIGH +5, MID −7, LOW +3 with the group's shapes), so the centre analysis (`current: None`, target = male LTASS) gives those three as `Rotate` (assert it). K = HIGH −2, MID +2, LOW −3; measured = raw + model(K). With `current: Some(&K)` every EQ knob is `Rotate` whose `db` is within 0.5 of the centre analysis's `db`, and `degrees == rotate_degrees(db − K, 10.0)`.
  - `small_change_turns_one_knob`: ProFX22v3 group 0 restore. K = 저역컷 1, HIGH 6, FREQ 800, MID −6, LOW −3. target = raw + model(K); measured = target − 5 × HighShelf(12 kHz) response (3 dB는 HIGH의 잡음 ✓ 범위 약 3.9 dB 안). → HIGH `Rotate` with db ∈ [10, 12] and degrees ∈ [40, 60]; FREQ, MID, LOW, 저역컷, COMP `Ok`.
  - `take_noise_alone_is_all_ok`: restore, measured = target + a fixed ±1 dB band pattern, on ProFX22v3 group 0 (K as above) and Signature 12 group 0 (60 Hz LOW; 저역컷 1, HIGH 4, FREQ 2500, MID −4, LOW −3) → every knob except GAIN `Ok` (the fixed 2.5 dB ✓ turned LOW 35° and 55°).
  - `one_real_change_keeps_the_others_in_noise_range`: V3 label retake with the fixed noise − 6 × HighShelf(12 kHz) → HIGH `Rotate` clockwise, LOW `Ok`.
  - `real_low_change_is_still_advised`: ProFX22v3 group 0, K as above, measured = target − 6 × LowShelf(80 Hz) → LOW `Rotate` with degrees > 0; HIGH, FREQ, MID, 저역컷 `Ok`. K = 저역컷 1, FREQ 125, MID −6, LOW −3, measured = target + 9 × LowShelf(80 Hz) → LOW `Rotate` with degrees < 0 (노브마다의 범위는 10 dB 남짓이라 d²로만 잡힘).
  - `normal_mode_entered_keeps_the_fixed_ok_range`: ProFX12v3 group 0 보통 모드, male LTASS + 4 × LowShelf(80 Hz): 가운데 분석의 LOW는 `Rotate`이고 `current: Some(&centre)`도 같은 동작.
  - `freq_only_move`: ProFX22v3 group 0 restore. target = raw + model(FREQ 2000, MID −6); K = FREQ 800, MID −6, measured = raw + model(K). → FREQ `SetFreq` with hz ∈ {1600, 2000, 2500}; MID `Ok`.
  - `entered_cut_and_comp_stay`: a summary where the normal-mode switch test is false and the COMP rule is true (crest ≥ 9, DR > 18). K 저역컷 1, COMP 30 → both `Ok`. K COMP 0 → COMP `Rotate { degrees: 30.0, .. }`. K COMP 15 → `Rotate { degrees: 15.0, .. }`.
  - `positions_helpers`: `centre(g)` fits; dropping one entry or renaming a label → not fits; `targets(&as_advice(g, &p), &centre(g)) == p` for p = ProFX22v3 group 0 (저역컷 1, HIGH 6, FREQ 800, MID −6, LOW −3, COMP 30); `freq_choices` of a sweep equals the engine's candidates (e.g. ProFX22v3 MID range) and is empty for a fixed MID.
- [ ] **Step 2:** `cargo test -q --lib advice` → FAIL (no `Positions`, no `current`).
- [ ] **Step 3:** Implement. Keep `knob_positions`, `neutral_summary`, `knobs_differ` for now (Task 3 removes them); they pass `current: None`.
- [ ] **Step 4:** `cargo test -q` and clippy → clean; all old engine tests unchanged.
- [ ] **Step 5:** `git commit -m "feat: advice from entered knob positions"`

### Task 2: 저장 파일에 노브 위치

**Files:** Modify `src/saved.rs` (`Saved`, `to_line`, `parse_line`, module doc, tests) and the minimum in `src/app.rs` to compile.

**Interfaces:**
- Consumes: `advice::Positions`, `advice::fits`, `advice::as_advice`.
- Produces: `Saved.knobs: Option<Positions>` replacing `Saved.flat`; `Flat` removed.
  - `to_line`: 31 fields (30 + `라벨=값;…`, values with f32 `Display`) only when `sample_rate` and `knobs` are both `Some`; else 30 or 29 as now.
  - `parse_line`: 29/30 → `knobs: None`; 31 → parse the knob field (each `;` item splits once at `=`, label non-empty, value a finite f32; an empty field or any bad item → bad line); 56 → the first 30 fields as now, the 26 centre-sound numbers must still parse as finite numbers, `knobs: None`; any other count → bad line.
- `app.rs` (this task only): the Stored view uses `s.knobs` (same mixer and group, `fits`) through `advice::as_advice`; `flat_for_save` and `NO_FLAT_SAVED` go; the save path writes `knobs: None` for now; everything else keeps compiling (the DIFFERS/Estimate code may stay until Task 3).

- [ ] **Step 1: Write the failing tests** in saved.rs:
  - `knobs_round_trip`: `Saved { sample_rate: Some(48000), knobs: Some(vec![("저역컷 100Hz", 1.0), ("HIGH 12kHz", 6.0), ("FREQ", 800.0), ("MID", -6.0), ("LOW 80Hz", -3.0), ("COMP", 30.0)] as Strings), .. }` → `to_line` has 31 tab fields and the last is `저역컷 100Hz=1;HIGH 12kHz=6;FREQ=800;MID=-6;LOW 80Hz=-3;COMP=30` → `parse` gives it back, no bad line.
  - `older_lines_have_no_knobs`: a 29-, a 30- and a 56-field line (0.4.0 format, from the old `flat_round_trip` data) parse with `knobs: None` and the right bands, no bad line.
  - `bad_knob_field`: 31-field lines whose last field is `HIGH=abc`, `HIGH`, `` (empty), `=3`, `HIGH=NaN` → each reported as a bad line.
  - `wrong_field_count_is_bad`: 32, 55 and 57 fields → bad line.
  - knobs but no sample rate → `to_line` writes 29 fields.
- [ ] **Step 2:** `cargo test -q --lib saved` → FAIL.
- [ ] **Step 3:** Implement; update the module doc and `to_line` doc ("29/30/31 fields; 56 read only").
- [ ] **Step 4:** `cargo test -q` and clippy → clean.
- [ ] **Step 5:** `git commit -m "feat: saved sounds carry knob positions instead of the centre sound"`

### Task 3: 입력 칸, 노브 줄, 저장

**Files:** Modify `src/app.rs` (Analysis, App, modal text, knob view, knob row/dial/caption, save path, tests) and `src/advice.rs` (remove the estimate fns).

**Interfaces:**
- Consumes: Task 1 `Positions`, `centre`, `fits`, `freq_choices`, `targets`, `as_advice`, `Inputs.current`; Task 2 `Saved.knobs`.
- Produces (pure, testable):
  - `Analysis.entered: Option<Positions>` (confirmed positions of an as-is analysis; `None` for a centre analysis or before 확인). `App.editing: Option<Positions>` (open entry form).
  - `fn current(a: &Analysis, group: &ChannelGroup) -> Option<Positions>` — centre analysis → `Some(centre(group))`; as-is → `entered` if it `fits`.
  - `advise(a, group, goal)` passes the confirmed positions of an as-is analysis and `None` for a centre analysis (same targets; keeps the fixed 2.5 dB ✓ of the centre analysis).
  - `enum KnobView { Advice(AdviceSet, Option<Positions>), Stored(Vec<Advice>), Entry, Names(String), None }` — the second field of `Advice` is the from-positions (`None` = centre).
  - `fn knob_view(analysis: Option<&Analysis>, editing: bool, saved: Option<&Saved>, mixer: &str, group: &ChannelGroup, goal: &Goal) -> KnobView` — spec rules 1–6.
  - `fn header(v: &KnobView) -> Option<&'static str>` — Advice centre `추천: 가운데(12시)에서 돌릴 양`; Advice entered `추천: 입력한 위치에서 돌릴 양`; Stored `저장된 세팅: 가운데(12시)에서 돌릴 양`; Entry `지금 노브 위치를 믹서 눈금대로 입력하고 [확인]을 누르세요.`; else `None`.
  - `fn prefill(view: &KnobView, group: &ChannelGroup) -> Positions` — Stored(k) → `targets(k, centre)`; Advice(s, from) → `targets(&s.knobs, from or centre)`; else centre; anything not `fits` → centre.
  - `fn saved_line(saved: Option<&Saved>, mixer: &str, group: &ChannelGroup) -> Option<String>` — the spec's `저장된 노브: …` line.
  - `fn save_positions(a: &Analysis, group: &ChannelGroup) -> Option<Positions>` — `current(a, group)`; `None` disables `이 소리 저장` with hover text `노브 위치를 입력하고 [확인]을 누르면 저장할 수 있습니다.`
  - `fn caption(k: &Advice, from: f32)`, `dial(ui, k, from: f32)`, `knob_row(ui, knobs, from: Option<&Positions>)` — from-value by label (0 when centre or missing). Captions per spec: EQ `Rotate` → [`{시계방향|반시계} {deg}°`, `({db:+.0}dB 눈금으로)`]; COMP `Rotate` → [`왼쪽 끝에서 {from+degrees:.0}°로`]; GAIN unchanged; others unchanged. Dial per spec (pointer at from, arc to target; FREQ ✓ with a known from-FREQ draws a dim pointer there; switch lit when from = 1 or `SwitchOn`; COMP pointer at −150° + from).
- Flow:
  - When an as-is analysis finishes and is usable (summary present, rate ≥ `MIN_RATE`, not silent): the fill is the open entry form if any, else the fill kept from an unusable as-is analysis, else `prefill(&view_before_this_analysis, group)` (computed before `readvise` replaces the view); `editing = Some(fill)`. Unusable → `editing = None` and the fill is kept for the next analysis (a centre analysis drops it): `entry_after(a, fill) -> (editing, kept)`.
  - Entry form (in the knob row area): per spec widgets; `확인` sits at the end of the header line, disabled while a MID ≠ 0 has FREQ unknown (`freq_missing`) with hover `FREQ 눈금을 고르세요.`; `확인` → `analysis.entered = editing.take()`, `readvise`. `입력 고치기` next to the entered header → `editing = analysis.entered.clone()`.
  - If `editing` does not `fits` the current group (mixer/group changed), replace it with `prefill` for the current group.
  - The `저장된 노브` line shows under the knob header whenever `saved_line` is `Some`.
  - Save stores `knobs: save_positions(..)`.
  - Modal weak lines: replace both with the spec's two lines verbatim; keep the three buttons.
  - Remove `LastFlat`/`last_flat`, `KnobView::Estimate`, `DIFFERS`, `NEED_FLAT`, `UNSURE`, and `advice::{knob_positions, neutral_summary, knobs_differ}` with their tests. Keep the engine behaviours they covered: rewrite `recovers_signature22_two_sweeps` and `finds_75hz_low_cut_under_a_low_mid_cut` (and any other kept behaviour) to call the engine directly (restore mode, measured = centre sound, target = finished sound) with the same expectations.
  - `overloaded` uses the same `current`.

- [ ] **Step 1: Write the failing tests** (app.rs tests, reuse `natural`/`saved_sample` helpers):
  - `knob_view_rules`: one assertion per rule 1–6 with exact headers: editing → `Entry`; centre analysis → `Advice(_, None)`; confirmed as-is → `Advice(_, Some)`; unconfirmed as-is is never `Advice`; unusable as-is (no summary; rate 22 050) → `Names(reason)`; no analysis + saved with knobs on the same mixer and group → `Stored`; saved from another mixer, another group, or without knobs → `None`.
  - `prefill_cases`: Stored → the stored positions; centre `Advice` with HIGH `Rotate { db: 6 }` → HIGH 6, rest 0; entered `Advice` with HIGH ✓ from K(HIGH 4) → HIGH 4; `None` → centre; positions of another group → centre.
  - `saved_line_text`: ProFX22v3 group 0 saved knobs (저역컷 1, HIGH 6, FREQ 800, MID −6, LOW −3, COMP 30) → `저장된 노브: 저역컷 켬 · HIGH 12kHz +6dB · FREQ 800Hz · MID -6dB · LOW 80Hz -3dB · COMP 30°`; all centre → `저장된 노브: 모두 가운데(저역컷·COMP 꺼짐)`; MID 0 with FREQ 800 → no FREQ item; another mixer → `None`.
  - `save_needs_positions`: centre → `Some(centre)`; as-is unconfirmed → `None`; confirmed → `Some(entered)`; entered for another group → `None`.
  - `centre_analysis_keeps_the_fixed_ok_range`: ProFX12v3 group 0 restore (saved = male LTASS, knobs centre), centre analysis of male LTASS + 4 × LowShelf(80 Hz) → LOW `Rotate` with degrees < 0 (centre positions would get the ~5 dB noise ✓).
  - `captions_from_position`: EQ `Rotate { degrees: 20, db: 6 }` → [`시계방향 20°`, `(+6dB 눈금으로)`]; COMP `Rotate { degrees: 15 }` from 15 → [`왼쪽 끝에서 30°로`]; GAIN unchanged (`(지금 위치에서)` line).
  - Update `flat_lines`/modal tests for the new weak lines.
- [ ] **Step 2:** `cargo test -q --bin mixer-analyzer knob_view` → FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** `cargo test -q` and clippy → clean. Build release, run `--demo`, and capture (see the earlier screenshot method in the dispatch): the entry form after a 그대로 analysis, and the knob row after 확인. Report the paths.
- [ ] **Step 5:** `git commit -m "feat: enter knob positions after an as-is analysis"`

### Task 4: README

**Files:** Modify `README.md`.

- [ ] **Step 1:** Per spec "README": 분석 항목(두 분석, 입력 칸, `입력 고치기`), `### 세팅 저장과 복구`(전문가 ①–④, 봉사자 흐름, `저장된 노브` 줄), 공유 폴더 "0.5.0 이상", and delete the 0.4.0 estimate sentences (LOW 오차, 두 분석 같은 사람·마이크, 지금 노브 추정). Then grep README for `추정`, `가운데 분석이 없`, `0.4.0`, `같은 사람` and fix any sentence the change contradicts.
- [ ] **Step 2:** `git commit -m "docs: entering knob positions"`
