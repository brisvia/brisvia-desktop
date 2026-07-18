# INCIDENT_PATTERNS.md — every error becomes a permanent defense

One entry per incident. No storytelling. The question that matters is **"why the controls that already
existed did not catch it"**, because that answer is the new defense.

Rule: an incident with no automated defense attached is **not closed**. "Remember to check X" is not a
defense.

---

### 1. The screen said 8 characters, the backend required 12
- **Invariant violated:** the message and the backend must apply the SAME rule.
- **Why it was not caught:** each half was correct on its own. The bug lives in the seam. Nobody ran
  "create a wallet" end to end.
- **Aggravating factor:** I diagnosed without reproducing ("it's a leftover message, just press Continue").
  Fernando wasted time confirming my wrong hypothesis.
- **Defense:** `product_first_contact` scenario A step 3 (test with one character LESS than the minimum).
- **Remaining risk:** the routine is manual until the E2E covers it.

### 2. Spanish text while the language was set to English
- **Invariant violated:** every visible text comes from the language system.
- **Why it was not caught:** I hand-wrote it into the HTML that same day. The i18n only sees what is marked
  with `data-i18n`: anything hand-written is invisible to it.
- **Defense:** `tools/check_textos.py` in CI (fails if there is hard-coded untranslated text; short, explicit
  allowlist).
- **Negative control:** tested by reintroducing the Spanish placeholder → it fails.

### 3. "You can create your wallet" on a screen only seen by someone who already has one
- **Invariant violated:** the text must be true FOR THAT STATE.
- **Why it was not caught:** no mechanical sweep sees it. The keys were complete, with no language mixing.
  It requires knowing who sees that screen and when.
- **Defense:** `PRODUCT_CONTRACTS.md` (FORBIDDEN texts per state) + `product_first_contact` scenario B.
- **Remaining risk:** the first time a text is absurd, a person has to see it; once written down as
  forbidden, it does not come back.

### 4. The app showed v0.4.0 while running 1.0.2
- **Invariant violated:** the version comes from ONE source: the build itself.
- **Why it was not caught:** three causes at once — `tauri.conf.json` and `Cargo.toml` out of sync
  (`app_version()` reads `CARGO_PKG_VERSION`), a hand-written `<span>v0.4.0</span>` as a fallback, and
  `checkForUpdate` reading the version FROM THE DOM.
- **Defense:** a lock in CI (tauri.conf == Cargo.toml == release tag) + zero hand-written versions +
  `runningVersion` as the only source.
- **Negative control:** tested with a fake tag (v1.0.9) → it fails.

### 5. I published 1.0.3 and broke the updater for EVERYONE
- **Invariant violated:** every public version must be detectable by the previous version.
- **Why it was not caught:** I generated the `latest.json` BY HAND. Nothing looked wrong: the page perfect,
  the installers downloading, the API saying "uploaded". Only the real URL showed the 404. **Reading
  GitHub's metadata is what fooled me.**
- **Defense:** `.github/workflows/publish-manifest.yml` (generated automatically on publish) +
  `tools/check_updater.py` (queries the public URL, just like the app).
- **Negative control:** tested by requesting a version that does not exist → it fails.

### 6. The 10,000-wallet battery had been red for HOURS
- **Invariant violated:** a suite must prove that it actually ran something.
- **Why it was not caught:** it did not even start (it died in the build script). Nobody was watching a
  workflow that had gone red silently, and "the tests pass" was something **believed, not checked**. It was
  broken across 3 already-published versions.
- **Defense:** `tools/run_tests_verified.py` (minimum number of tests + zero failures + not suspiciously
  fast).
- **Negative control:** reproduced — `cargo test` with a filter that matches nothing comes out **GREEN
  having tested nothing**.

### 7. I corrupted the node database on Fernando's machine, TWICE
- **Invariant violated:** use the cheapest available environment; clean shutdown before forcing.
- **Why it was not caught:** there was nothing to prevent it. I killed the process with `Stop-Process -Force`.
- **Defense:** the self-repair (`classify_failure` + causality from the current attempt's log) + Rule 5.
- **Good side effect:** the bug forced me to write the repair, which later worked in real life.

### 8. The anti-loop marker was not cleared after a successful repair
- **Invariant violated:** a protection mechanism cannot leave the user worse off than not having it.
- **Why it was not caught:** the repair WORKED. The bug only appeared the **second** time (every update
  force-closes the app, which is what corrupts the database). The tests were green.
- **How it was found:** I went and looked at Fernando's machine after he updated, instead of assuming.
- **Defense:** a dated marker (recent = loop, old = closed incident) + 4 tests covering that sequence.

### 9. I opened test windows on top of a running game
- **Invariant violated:** environment hierarchy (runner > temporary > development > Fernando's machine).
- **Why it was not caught:** no rule forbade it. And **the same tester was already running in the cloud**,
  where it had passed that very morning.
- **Defense:** Rule 5. The E2E runs on the Windows runner, never locally.

### 10. I tried to copy a credentials file into a PUBLIC repo
- **Invariant violated:** secrets never enter a public Git tree, not even temporarily.
- **Why it was not caught:** an external classifier stopped it, not me. **The operation should never have
  gotten that far.**
- **Defense:** Rule 5 + a strict `.gitignore` (client_secret, token, gsc_auth, *.pem, *.key) + pointing
  scripts at `C:\secure\fernando-secrets` instead of copying.
- **Remaining risk:** the separation depends on .gitignore. A file with a new name would slip through.

---

## The pattern behind the 10

A root-cause analysis grouped them into **5 causes, not 10 problems** (ROOT_CAUSE 2026-07-14):

- **A. Nobody owned the whole product** → incidents 1, 2, 3, 4. The reviewers checked pieces; none walked
  through "I am a person opening Brisvia for the first time".
- **B. Evidence neither observed nor demanded** → 5, 6. Green/published accepted without demanding proof of
  the effect.
- **C. Destructive without isolation** → 7, 9. I acted on the most valuable environment while cheaper ones
  were available.
- **D. Security boundaries outside the normal flow** → 10.
- **E. Diagnosis by intuition before reproducing** → 1.

> *"You turn the absence of a visible error into evidence of success."*
>
> The workflow existed → I assumed it tested. The release had files → I assumed it updated. The code
> compiled → I assumed it showed the correct version. The repair finished → I assumed the state was left fine.
