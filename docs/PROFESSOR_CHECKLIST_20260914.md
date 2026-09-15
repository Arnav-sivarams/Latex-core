# Professor checklist — 2026-09-14

Evidence was produced against uniquely named disposable PostgreSQL databases, the real Axum server, headless Chromium, and the unchanged frozen M7 image. Commands below are rerunnable; secrets and runtime logs are deliberately not committed. A source/unit assertion is not labelled as browser evidence.

| Item | Requirement | Implementation / files | Executable test evidence | Status | Remaining issue |
| ---: | --- | --- | --- | --- | --- |
| 1 | First-use Front Matter configuration and managed `Front-Matter.tex` | `frontend/document-details.mjs`, `frontend/writer.js`, `persistence/front_matter.rs`, migration 0028 | `FRONTMATTER_BROWSER=1 cargo test -p server --features database-tests --bin latex-core-api legacy_front_matter_institutional_api_contract -- --nocapture` verifies one-time UI, reload, DB values, managed file and real PDF | PASS | None found in exercised separate-file/VIT path |
| 2 | User-driven compile only | Existing compile barrier plus metadata save changes in `main.rs`; no save path submits jobs | Same browser/M7 test observes zero jobs after metadata saves, then two explicit builds; `cargo test -p persistence --features database-tests --test v2_domain manual_build_review_lock_and_integration_reads_share_exact_state` | PASS | No claim that this addresses unrelated performance issues |
| 3 | Writer/Mentor turn-taking | Existing review round/capability/epoch gates; browser journey extended in `front-matter-browser.mjs` | Browser keeps second Writer open, observes both Writers lock, first of two Mentor pushes keeps the round open, final Push unlocks Writers; focused persistence/realtime tests cover direct HTTP/Yjs writes, drafts and stale sessions | PASS | Aggregate server DB suite was not rerun after the focused fixture repair |
| 4 | Selected-range editing/undo/collaboration | `insertLatex` now captures Yjs relative positions, resolves them after dialogs, rejects unavailable/file-changed ranges, and uses tracked undo transactions | Real two-Writer Chromium journey replaces part of a line, verifies the second Writer's exact text, undoes it, then inserts after a concurrent remote prefix without replacing unrelated text | PASS | None found in the exercised collaboration path |
| 5 | Image upload, insertion, PDF | Upload retains the active editor, uses wrapper/main-aware destinations, and rebases figure paths relative to the compilation root | Chromium uploads a generated real PNG to `Thesis_content_v1.0/images/browser.png`, inserts `images/browser.png`, manually compiles with frozen M7, and verifies the PDF contains an image paint operator | PASS | No pixel snapshot is committed; binary rendering was inspected through PDF operators |
| 6 | Code block insertion | Unified Insert uses `listings`; `buildCodeListing` preserves literal multiline input | Chromium inserts indented code with special characters through the dialog; the real M7 compile succeeds and extracted PDF text contains the listing | PASS | None found |
| 7 | Multiline source comments | `commentLatexLines` retains its tracked Yjs transaction; `review.js` records multiline anchor line ranges and preserves multiline message bodies | Existing source comment/uncomment/undo Chromium evidence plus focused Chromium Mentor mouse-drag across two lines; published two-paragraph comment retained | PASS | None found in exercised cases |
| 8 | Obvious inline math | Unified Insert entry and delimiter-aware `inlineMathInsertion` share the Yjs-relative insertion path | Chromium inserts inline math at a preserved selection, inserts a grid symbol, and the combined M7 document compiles | PASS | None found |
| 9 | One Insert catalogue | `openInsertMenu` in `frontend/writer.js`; legacy controls hidden | Real browser asserts exactly one Table/Figure/Code/Inline/Display/Symbols/Publications entry | PASS | None found |
| 10 | Table caption above | `buildTable` emits caption/label before `tabular`; longtable retains valid top caption | Chromium generates a nonuniform-width table through the UI and the combined M7 document compiles with its caption | PASS | None found for ordinary table generation |
| 11 | Figure caption below | `buildFigure` emits the image before its caption | Chromium generates the figure through the UI; source ordering, caption text, and painted PDF image are verified after manual M7 compile | PASS | None found |
| 12 | No duplicated Insert commands in ellipsis | `commands()` exposes only one `Insert…` bridge, not individual insert functions | Real browser asserts individual insert actions absent from More actions | PASS | None found |
| 13 | Mentor PDF wider than source | `.mentor-shell .three-pane-workspace` role-only grid in `static/shells.css` | Real browser compares rendered source/PDF bounding boxes | PASS | No draggable pane resizer exists to persist a custom split |
| 14 | Annotation opens exact file/range or truthful fallback | Stable file ID + Yjs-relative anchors, persisted reviewed start/end lines, quoted-text verification, and disabled Open action after drift | Focused Chromium creates/publishes a second-file multiline annotation with repeated phrases, opens the exact range, then changes source and verifies excerpt/location fallback plus disabled Open | PASS | None found in exercised exact/fallback cases |
| 15 | Chapter destination | Wrapper/main-aware `insertionDirectories`, durable-flush precondition, and confirmed complete destination | Chromium uses the real New File prompts and creates `Thesis_content_v1.0/chapters/chapter9.tex` after collaborative edits | PASS | None found |
| 16 | Image/asset destination | Resolver prefers actual `images/`/`assets/`; insertion rebases report paths against the nested main file | Chromium uploads into `Thesis_content_v1.0/images/`, inserts the relative `images/browser.png`, and compiles successfully | PASS | None found |
| 17 | Categorized publications | Advertised Publications action opens typed project metadata; guarded categorized `\bibitem` generator checks both pending entries and existing report keys | Chromium opens Publications from Insert, inserts communicated/accepted/published entries, compiles all three labels, and confirms a repeated insertion is rejected | PASS | None found |
| 18 | Table-cell sizing | Table Builder selects a cell, edits its containing column width/row minimum height, and explains shared geometry; dimensions remain bounded | Frontend positive/negative tests and focused Chromium compile of `p{3cm}p{5cm}`, selected-row `8mm`, multiline `shortstack`, top caption | PASS | None found in exercised cases |
| 19 | PDF reading position and source location | Local PDF.js multi-page viewer stores page/zoom/within-page offset per report, suppresses transient render scroll capture, clamps, and gates SyncTeX to a current build | Focused Chromium scrolls inside page 3 to 42%, manually rebuilds and observes page 3/42%; exact Locate reaches page 3; another report opens at page 1; a later one-page build clamps to page 1 | PASS | None found in exercised cases |
| 20 | Search in header | Search markup moved to header; explorer has no duplicate | Real browser asserts header placement and explorer absence | PASS | None found |
| 21 | Centred login logo | `.login-brand` flex centering, contained responsive image | Real browser checks computed `justify-content:center` and `object-fit:contain` before login | PASS | Custom uploaded-logo visual snapshot not retained |
| 22 | Three template arrangements | Existing arrangements retained; recognized institutional `SINGLE_SOURCE` uses one verified marker after known macro declarations to load report-local managed definitions without changing the immutable original | Unit compatibility tests, frozen-M7 compiler test, and focused Chromium: two reports from one GUI-imported template compile distinct names/title/Guide; form change updates only one report; Rust fixture verifies original unchanged and one binding/report | PASS | Arbitrary unrecognized TeX remains explicitly unsupported by design |
| 23 | All requested project metadata | Migration 0029 adds project-scoped Department/School display-name collections; resolver retains canonical IDs, per-value provenance, managed TeX, captured versions, and API projection | Focused DB/API contract uses two Departments/Schools and verifies save/reload/materialization/capture/export/no build; Chromium verifies form reload, updated PDF, API value, and zero-build GET | PASS | VCAP identities without global names still require truthful Leader input per report |
| 24 | Accessible symbol grid | Catalogue is a searchable keyboard-accessible grid with names/tooltips; insertion shares the relative-selection path and wraps symbols safely outside math | Chromium verifies grid accessibility/navigation and inserts a symbol at the preserved selection; the combined M7 document compiles | PASS | Rice reference PDF contents were not reviewed in this environment |

## Qualification results

- Closing browser/M7: `professor_remaining_single_source_annotations_and_pdf_browser` — **1 passed, 0 failed, 50 filtered**; output reported two report-local single-source builds, metadata save/reload/API, compiled selected-cell table, page 3 at 42% after rebuild, one-page clamp, exact Locate page 3, multiline second-file exact/fallback annotation, and zero browser errors.
- Project metadata DB/API: `legacy_front_matter_institutional_api_contract` — **1 passed, 0 failed, 50 filtered** on its own disposable database.
- Review lifecycle: `s5_review_authorization_lifecycle_suggestions_rounds_and_isolation` — **1 passed, 0 failed, 47 filtered**.
- Review lock/exact state: `manual_build_review_lock_and_integration_reads_share_exact_state` — **1 passed, 0 failed, 6 filtered**.
- Institutional credentials/scopes/read-only routes: `institutional_api_admin_lifecycle_scope_and_read_only_routes` — **1 passed, 0 failed, 47 filtered**.
- Realtime collaboration fixture: `realtime_collaboration_converges_persists_recovers_and_enforces_access` — **1 passed, 0 failed, 47 filtered** against its own disposable database.
- Frontend: `npm test` — **11 test files passed**.
- Source gates: `cargo test --workspace`, `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `git diff --check`, OpenAPI semantic checks, and `docker compose ... config --quiet` — **passed**.
- The full aggregate server database suite was not rerun after making the realtime fixture self-contained; the focused database tests above used separate, uniquely named disposable databases.

## Delivery classification

- **Implemented and tested:** Yjs-relative dialog insertion and undo, two-Writer selected editing, nested chapter creation, real PNG upload and relative figure insertion, code/math/symbol/table/figure/publication insertion, duplicate publication-key rejection, and combined manual frozen-M7 compilation.
- **Implemented and tested:** all 24 rows now have executable evidence. The six closing items are covered by `professor_remaining_single_source_annotations_and_pdf_browser`, focused DB/API contracts, frontend tests, and the frozen-M7 single-source compile.
- **Implemented but not tested:** none among Items 7, 14, 18, 19, 22, and 23.
- **Not implemented:** universal rewriting of arbitrary single-source TeX (explicitly outside the bounded contract).

## Review transition table

| State/event | Writers | Leader | Mentors | Next state |
| --- | --- | --- | --- | --- |
| Editable | Existing role/file-policy mutations and manual Compile | Same, plus Send after exact-PDF check | Read only; cannot draft | Send opens review |
| Open for review | Read/navigation only; HTTP/Yjs/compile/feedback mutation denied | Read/navigation plus End review only | Assigned pending Mentors draft privately and Push | Early Push publishes that Mentor only; stays open while required Mentors remain |
| Final required Push | Normal prior permissions restored | Normal prior permissions restored | Submitted feedback published; drafts belonging to others are not exposed | Round completed |
| Leader End review | Normal prior permissions restored | Performs closure | Unpublished drafts remain private | Round completed/withdrawn |

## Evidence locations

- Browser journey: `services/server/tests/front-matter-browser.mjs`
- Six-item closing browser journey: `services/server/tests/professor-remaining-browser.mjs`
- DB/API setup and assertions: `services/server/src/front_matter_db_tests.rs`
- Review/exact-state contract: `crates/persistence/tests/v2_domain.rs`
- Institutional lifecycle contract: `services/server/src/main.rs` (`institutional_api_admin_lifecycle_scope_and_read_only_routes`)
- API client: `examples/institutional-archive/`
