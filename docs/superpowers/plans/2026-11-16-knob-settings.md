# 소리 + 노브 세팅 함께 저장 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 저장한 세팅이 소리(완성 대역)와 가운데 소리(raw 대역)를 함께 갖고, 노브 위치는 엔진 복구 계산으로 얻어 노브 줄에 보여 준다.

**Architecture:** `saved.rs`에 선택 필드 `flat`(가운데 DR + 25대역)을 더해 56필드로 쓴다. `advice.rs`에 노브 위치 계산(엔진 복구 모드 호출, GAIN ✓, 노트 버림)과 차이 판정을 둔다. `app.rs`는 분석 종류(`flat`), 안내창 세 번째 버튼, `last_flat`, 노브 줄 규칙 1–5와 저장 규칙을 순수 함수로 두고 UI에서 부른다.

**Tech Stack:** Rust 1.96, eframe/egui 0.37.

**Spec:** `docs/superpowers/specs/2026-11-16-knob-settings-design.md` (flat-start: `docs/superpowers/specs/2026-11-16-flat-start-design.md` 위에 얹음)

## Global Constraints
- spec의 한국어 문구(버튼, 머리글, 노트)는 글자 그대로.
- 빌드·테스트는 `CARGO_TARGET_DIR=/private/tmp/claude-501/-Users-sli/6b230aad-5cf4-4c59-a8ab-cfdc1c61b35f/scratchpad/vis-target`. `cargo test -q`와 `cargo clippy -q --all-targets`가 깨끗해야 한다.
- 커밋은 제목 한 줄(본문·트레일러 없음), conventional prefix. 저장소 루트의 스크린샷 PNG 두 개는 커밋하지 않는다.
- 브랜치 `feat/knob-settings`(`feat/flat-start` 위에서 시작; 둘을 함께 main에 머지).

## Review Focus
1. 믹서나 그룹을 바꾼 뒤 남아 있는 `last_flat`: 같은 믹서·그룹·샘플레이트일 때만 쓰여야 한다(Task 3 테스트).
2. 노브 위치가 없는 옛 저장(29·30필드): 0.3.x처럼 소리 기준으로만 동작해야 한다(Task 1·3 테스트).
3. 다른 믹서·그룹에서 저장한 세팅을 고름: 저장된 노브 위치를 보여 주지 않아야 한다(규칙 2의 일치 조건, Task 3 테스트).
4. 가운데 분석 직후 저장: 노브 위치가 모두 ✓여야 한다(Task 2 테스트).
5. 그대로 분석인데 `last_flat`이 없음: 노브 이름과 안내 노트만, 잘못된 추천이 없어야 한다(Task 3 테스트).

---

### Task 1: 저장 파일에 가운데 소리

**Files:**
- Modify: `src/saved.rs` (`Saved`, `to_line`, `parse_line`, tests)
- Modify: every `Saved { .. }` literal in `src/app.rs` (add `flat: None` or the real value later in Task 3)

**Interfaces:**
- Produces: `pub struct Flat { pub dynamic_range_db: f32, pub bands_db: [f32; 25] }` (`Clone, Debug, PartialEq`), `Saved.flat: Option<Flat>`.

- [ ] **Step 1: Write the failing tests** in `saved.rs` tests:
  - `flat_round_trip`: `Saved { sample_rate: Some(48000), flat: Some(Flat { dynamic_range_db: 12.5, bands_db: [-30.0; 25] }), .. }` → `to_line` has 56 tab fields → `parse` gives it back, `None` bad line.
  - `older_lines_have_no_flat`: 29- and 30-field lines parse with `flat: None`.
  - `wrong_field_count_is_bad`: a 31-field and a 55-field line → reported as a bad line.
- [ ] **Step 2:** `cargo test -q flat` → FAIL (no `Flat`).
- [ ] **Step 3:** Implement. `to_line` writes the 26 flat numbers only when both `sample_rate` and `flat` are `Some`. Update the module doc comment and the `to_line` doc ("29/30/56 fields").
- [ ] **Step 4:** `cargo test -q` → all pass.
- [ ] **Step 5:** `git commit -m "feat: saved sounds can carry the centre (flat) sound"`

### Task 2: 노브 위치 계산과 차이 판정

**Files:**
- Modify: `src/advice.rs` (new fns + tests)
- Modify: `src/speech.rs` only if the neutral summary belongs there (implementer's choice, shorter wins)

**Interfaces:**
- Consumes: `AdviceEngine::advise`, `Inputs { restore_dynamic_range_db: Some(final_dr), .. }`, `Summary`.
- Produces:
  - `pub fn knob_positions(group: &ChannelGroup, flat: &Summary, final_bands: &[f32; 25], final_dr: f32) -> Vec<Advice>`: one fresh engine in restore mode (summary = flat, target = final_bands); GAIN forced to `Action::Ok`; notes dropped.
  - `pub fn neutral_summary(bands_db: [f32; 25], dynamic_range_db: f32) -> Summary`: for saved flats; values chosen so the engine gives no GAIN/COMP-from-crest/overload/noise side effects (peak −12, rms −24, crest 12, noise floor −70, snr 46, active 20 s, empty bin_power), each documented in one line.
  - `pub fn knobs_differ(a: &[Advice], b: &[Advice]) -> bool`: true if, for the same label, an EQ `Rotate` differs by ≥ 3 dB (`Ok` counts as 0 dB), a `SetFreq` differs by more than a factor 2^(1/3), or one is `SwitchOn` and the other is not. Labels absent from either side are ignored.

- [ ] **Step 1: Write the failing tests:**
  - `recovers_known_settings`: on ProFX22v3 CH1-12, flat = Byrne male LTASS (`ltass_db(Voice::Male)`) through `neutral_summary`. final = flat + HIGH shelf +6 at 12 kHz + MID bell −6 at 800 Hz + LOW shelf −3 at 80 Hz + `hpf_db(100.0, f)`, using the same shape functions the engine uses. Assert: HIGH db ∈ [5, 7], FREQ ∈ {630, 800, 1000}, MID db ∈ [−7, −5], LOW db ∈ [−4, −2], 저역컷 `SwitchOn`, GAIN `Ok`.
  - `flat_saved_right_after_flat_analysis_is_all_ok`: final = flat → every knob `Ok`.
  - `comp_from_dynamic_range`: flat DR 18, final DR 12 → COMP `Rotate { degrees: 30.0, .. }`. flat DR 12, final DR 12 → COMP `Ok`.
  - `knobs_differ_thresholds`: 2.5 dB → false. 3 dB → true. FREQ 800 vs 1000 → false. FREQ 800 vs 1250 → true. SwitchOn vs Ok → true.
- [ ] **Step 2:** `cargo test -q --lib advice` → FAIL.
- [ ] **Step 3:** Implement the three fns.
- [ ] **Step 4:** `cargo test -q` and clippy → clean.
- [ ] **Step 5:** `git commit -m "feat: knob positions from a centre sound and a finished sound"`

### Task 3: 분석 두 가지, 노브 줄 규칙, 저장 규칙

**Files:**
- Modify: `src/app.rs` (`Analysis`, the flat-start modal, `App` fields, `readvise`/`advise` call sites, knob-row header, save path, tests)

**Interfaces:**
- Consumes: Task 1 `Flat`/`Saved.flat`, Task 2 fns, flat-start's modal and `flat_lines`.
- Produces (pure, testable):
  - `Analysis.flat: bool`.
  - `struct LastFlat { mixer: String, group: String, sample_rate: f32, summary: Summary }`, field `App.last_flat: Option<LastFlat>`, set when a 가운데 analysis with a summary completes.
  - `enum KnobView { Advice(AdviceSet), Stored(Vec<Advice>), Estimate(Vec<Advice>), Names(String /*note*/), None }` and `fn knob_view(analysis: Option<&Analysis>, saved: Option<&Saved>, last_flat: Option<&LastFlat>, mixer: &str, group: &ChannelGroup, goal: &Goal) -> KnobView`, which follows spec rules 1–5. Rule 1 appends the difference note when `knobs_differ` holds.
  - `fn header(v: &KnobView) -> Option<&'static str>` returns the three spec headers. This replaces flat-start's static hint label.
  - `fn flat_for_save(analysis: &Analysis, last_flat: Option<&LastFlat>, mixer: &str, group: &str) -> Option<Flat>` follows the spec's three save cases. After a save with `None`, show the spec note in the same red/notice line the save path already uses.

- [ ] **Step 1: Write the failing tests** (app.rs tests, reuse the existing `natural`/`saved_sample` helpers):
  - `knob_view_rules`: one assertion per rule 1–5. Include last_flat for another group → rule 4, and a saved setting from another mixer → not rule 2.
  - `difference_note_only_when_different`.
  - `flat_for_save_cases`: flat analysis → `Some` equal to its own bands. 그대로 + matching last_flat → last_flat's. 그대로 + mismatched rate → `None`.
- [ ] **Step 2:** `cargo test -q --bin mixer-analyzer knob_view` → FAIL.
- [ ] **Step 3:** Implement.
  - Modal: add the `지금 세팅 그대로 분석` button and the extra weak line from the spec. Enter stays 가운데. The recording carries the kind into `Analysis.flat`.
  - The knob row uses `knob_view` and `header`.
  - Saving uses `flat_for_save`.
  - Apply the spec's "flat-start 리뷰에서 넘어온 것" section:
    - COMP caption reads `왼쪽 끝에서 시계방향 30°`, and the dial's pointer and arc start at −150°, including COMP Ok.
    - GAIN Rotate gets a `(지금 위치에서)` line.
    - The dialog line reads `GAIN은 그대로`.
    - Update the `flat_lines` test and the caption tests to match.
- [ ] **Step 4:** `cargo test -q` and clippy → clean. Build release and, if possible, screenshot the modal with three buttons (see flat-start's method). Report the path.
- [ ] **Step 5:** `git commit -m "feat: centre and as-is analyses, saved knob positions in the knob row"`

### Task 4: README

**Files:** Modify `README.md`.

- [ ] **Step 1:** Make these README changes:
  - In the 분석 bullet, explain the two kinds of analysis.
  - Add a "세팅 저장과 복구" paragraph under `### 목표` or the save-folder section. Expert steps: ① 가운데로 맞추고 분석 ② 추천대로 돌리고 귀로 다듬기 ③ 지금 세팅 그대로 분석 ④ 이 소리 저장. Volunteer steps: same mixer and channel → the stored knob positions appear right away; different mic, voice or mixer → centre analysis.
  - Change the shared-folder line to "0.4.0 이상".
  - Add `(GAIN은 지금 위치에서, COMP는 왼쪽 끝에서)` to the angle sentence.
  - Then grep for sentences the change contradicts.
- [ ] **Step 2:** `git commit -m "docs: saving and restoring knob settings"`
