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
| 7 | Multiline source comments | `commentLatexLines` uses one tracked Yjs transaction and relative selection; Mentor multiline model unchanged | Chromium comments/uncomments a partial multiline selection including a blank line and verifies undo/redo; frontend unit coverage also passes | PARTIAL | Multiline Mentor annotation ranges were not re-exercised in this pass |
| 8 | Obvious inline math | Unified Insert entry and delimiter-aware `inlineMathInsertion` share the Yjs-relative insertion path | Chromium inserts inline math at a preserved selection, inserts a grid symbol, and the combined M7 document compiles | PASS | None found |
| 9 | One Insert catalogue | `openInsertMenu` in `frontend/writer.js`; legacy controls hidden | Real browser asserts exactly one Table/Figure/Code/Inline/Display/Symbols/Publications entry | PASS | None found |
| 10 | Table caption above | `buildTable` emits caption/label before `tabular`; longtable retains valid top caption | Chromium generates a nonuniform-width table through the UI and the combined M7 document compiles with its caption | PASS | None found for ordinary table generation |
| 11 | Figure caption below | `buildFigure` emits the image before its caption | Chromium generates the figure through the UI; source ordering, caption text, and painted PDF image are verified after manual M7 compile | PASS | None found |
| 12 | No duplicated Insert commands in ellipsis | `commands()` exposes only one `Insert…` bridge, not individual insert functions | Real browser asserts individual insert actions absent from More actions | PASS | None found |
| 13 | Mentor PDF wider than source | `.mentor-shell .three-pane-workspace` role-only grid in `static/shells.css` | Real browser compares rendered source/PDF bounding boxes | PASS | No draggable pane resizer exists to persist a custom split |
| 14 | Annotation opens exact file/range or truthful fallback | Stable file ID + relative anchor resolution and reviewed-PDF fallback in `focusWriterReview` | Existing server review test covers stable review identity/privacy; second-file repeated-phrase browser case not run | PARTIAL | Run the specified second-file repeated-text browser case |
| 15 | Chapter destination | Wrapper/main-aware `insertionDirectories`, durable-flush precondition, and confirmed complete destination | Chromium uses the real New File prompts and creates `Thesis_content_v1.0/chapters/chapter9.tex` after collaborative edits | PASS | None found |
| 16 | Image/asset destination | Resolver prefers actual `images/`/`assets/`; insertion rebases report paths against the nested main file | Chromium uploads into `Thesis_content_v1.0/images/`, inserts the relative `images/browser.png`, and compiles successfully | PASS | None found |
| 17 | Categorized publications | Advertised Publications action opens typed project metadata; guarded categorized `\bibitem` generator checks both pending entries and existing report keys | Chromium opens Publications from Insert, inserts communicated/accepted/published entries, compiles all three labels, and confirms a repeated insertion is rejected | PASS | None found |
| 18 | Table-cell sizing | Builder exposes bounded column widths and one shared minimum row height with an accurate geometry explanation | Chromium compiles a 3 cm/6 cm nonuniform table with multiline content and 8 mm minimum row height | PARTIAL | There is no selected-cell control that edits its containing column and row |
| 19 | PDF reading position and source location | Per-report page/zoom/offset persistence and exact-build SyncTeX action in `frontend/writer.js` | Existing M7 browser proves PDF refresh; position and source-location actions were not exercised | PARTIAL | Run both required browser assertions |
| 20 | Search in header | Search markup moved to header; explorer has no duplicate | Real browser asserts header placement and explorer absence | PASS | None found |
| 21 | Centred login logo | `.login-brand` flex centering, contained responsive image | Real browser checks computed `justify-content:center` and `object-fit:contain` before login | PASS | Custom uploaded-logo visual snapshot not retained |
| 22 | Three template arrangements | Migration 0028 stores all arrangements; separate-file rendering now supports a bounded `../` marker for a verified nested main file and rebases only the generated wrapper | Chromium proves the wrapped separate-file case compiles; classification tests cover all three arrangement values | PARTIAL | A recognized single-source institutional template is still metadata-only; verified macro binding is not implemented |
| 23 | All requested project metadata | Typed table/DTOs in migration 0028 and `front_matter.rs`; browser editor and integration projection | Focused DB/API contract verifies executive-summary distinction, type, datasets, literal code, publications, canonical people/provenance, managed TeX and zero builds | PARTIAL | Department/school display names remain unavailable where VCAP supplies identities without names; no narrowly scoped override input was added |
| 24 | Accessible symbol grid | Catalogue is a searchable keyboard-accessible grid with names/tooltips; insertion shares the relative-selection path and wraps symbols safely outside math | Chromium verifies grid accessibility/navigation and inserts a symbol at the preserved selection; the combined M7 document compiles | PASS | Rice reference PDF contents were not reviewed in this environment |

## Qualification results

- Browser/M7: `legacy_front_matter_institutional_api_contract` — **1 passed, 0 failed, 47 filtered**; output reported Leader/Writer/editor insertions/wrapped paths/uploaded PDF image/publications/Mentor/review turn-taking passed, 27 populated PDF assertions, two manual compiles, and zero browser errors.
- Review lifecycle: `s5_review_authorization_lifecycle_suggestions_rounds_and_isolation` — **1 passed, 0 failed, 47 filtered**.
- Review lock/exact state: `manual_build_review_lock_and_integration_reads_share_exact_state` — **1 passed, 0 failed, 6 filtered**.
- Institutional credentials/scopes/read-only routes: `institutional_api_admin_lifecycle_scope_and_read_only_routes` — **1 passed, 0 failed, 47 filtered**.
- Realtime collaboration fixture: `realtime_collaboration_converges_persists_recovers_and_enforces_access` — **1 passed, 0 failed, 47 filtered** against its own disposable database.
- Frontend: `npm test` — **11 test files passed**.
- Source gates: `cargo test --workspace`, `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `git diff --check`, OpenAPI semantic checks, and `docker compose ... config --quiet` — **passed**.
- The full aggregate server database suite was not rerun after making the realtime fixture self-contained; the focused database tests above used separate, uniquely named disposable databases.

## Delivery classification

- **Implemented and tested:** Yjs-relative dialog insertion and undo, two-Writer selected editing, nested chapter creation, real PNG upload and relative figure insertion, code/math/symbol/table/figure/publication insertion, duplicate publication-key rejection, and combined manual frozen-M7 compilation.
- **Implemented but not fully tested:** stable annotation identities/fallback UI and outer PDF page/zoom/offset persistence exist, but the required repeated-text second-file click, internal PDF.js scroll preservation, and exact Locate source action were not exercised.
- **Not implemented:** verified single-source macro binding, department/school display-name overrides where VCAP lacks names, and selected-cell sizing controls.

## Review transition table

| State/event | Writers | Leader | Mentors | Next state |
| --- | --- | --- | --- | --- |
| Editable | Existing role/file-policy mutations and manual Compile | Same, plus Send after exact-PDF check | Read only; cannot draft | Send opens review |
| Open for review | Read/navigation only; HTTP/Yjs/compile/feedback mutation denied | Read/navigation plus End review only | Assigned pending Mentors draft privately and Push | Early Push publishes that Mentor only; stays open while required Mentors remain |
| Final required Push | Normal prior permissions restored | Normal prior permissions restored | Submitted feedback published; drafts belonging to others are not exposed | Round completed |
| Leader End review | Normal prior permissions restored | Performs closure | Unpublished drafts remain private | Round completed/withdrawn |

## Evidence locations

- Browser journey: `services/server/tests/front-matter-browser.mjs`
- DB/API setup and assertions: `services/server/src/front_matter_db_tests.rs`
- Review/exact-state contract: `crates/persistence/tests/v2_domain.rs`
- Institutional lifecycle contract: `services/server/src/main.rs` (`institutional_api_admin_lifecycle_scope_and_read_only_routes`)
- API client: `examples/institutional-archive/`
