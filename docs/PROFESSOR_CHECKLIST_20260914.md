# Professor checklist — 2026-09-14

Evidence was produced against uniquely named disposable PostgreSQL databases, the real Axum server, headless Chromium, and the unchanged frozen M7 image. Commands below are rerunnable; secrets and runtime logs are deliberately not committed. A source/unit assertion is not labelled as browser evidence.

| Item | Requirement | Implementation / files | Executable test evidence | Status | Remaining issue |
| ---: | --- | --- | --- | --- | --- |
| 1 | First-use Front Matter configuration and managed `Front-Matter.tex` | `frontend/document-details.mjs`, `frontend/writer.js`, `persistence/front_matter.rs`, migration 0028 | `FRONTMATTER_BROWSER=1 cargo test -p server --features database-tests --bin latex-core-api legacy_front_matter_institutional_api_contract -- --nocapture` verifies one-time UI, reload, DB values, managed file and real PDF | PASS | None found in exercised separate-file/VIT path |
| 2 | User-driven compile only | Existing compile barrier plus metadata save changes in `main.rs`; no save path submits jobs | Same browser/M7 test observes zero jobs after metadata saves, then two explicit builds; `cargo test -p persistence --features database-tests --test v2_domain manual_build_review_lock_and_integration_reads_share_exact_state` | PASS | No claim that this addresses unrelated performance issues |
| 3 | Writer/Mentor turn-taking | Existing review round/capability/epoch gates; browser journey extended in `front-matter-browser.mjs` | Browser keeps second Writer open, observes both Writers lock, first of two Mentor pushes keeps the round open, final Push unlocks Writers; focused persistence/realtime tests cover direct HTTP/Yjs writes, drafts and stale sessions | PASS | Aggregate server DB suite was not rerun after the focused fixture repair |
| 4 | Selected-range editing/undo/collaboration | Common CodeMirror/Yjs `insertLatex` captures stable selection and transacts ranges in `frontend/writer.js` | Frontend tests pass; live two-Writer selected-range mouse/keyboard journey not executed | PARTIAL | Add browser selection/replace/undo assertion |
| 5 | Image upload, insertion, PDF | Report-scoped upload, capability recheck, explicit replace/rename and figure builder in `frontend/writer.js` | Generator/path unit tests and existing server asset contract; no browser upload-to-real-PDF in this pass | PARTIAL | Execute PNG upload, insert and visual/PDF assertion |
| 6 | Code block insertion | Unified Insert uses `listings`; `buildCodeListing` preserves literal multiline input | `node --test frontend/*.test.mjs` verifies listing without minted/shell escape | PARTIAL | Browser insertion and M7 rendering not exercised |
| 7 | Multiline source comments | `commentLatexLines` plus one Yjs transaction; Mentor multiline model unchanged | Frontend unit test covers partial content/blank lines/comment/uncomment | PARTIAL | Browser selection/undo and multiline Mentor range not re-exercised |
| 8 | Obvious inline math | Unified Insert entry and delimiter-aware `inlineMathInsertion` | Frontend unit tests cover wrapping and already-inside detection | PARTIAL | Browser insertion/undo not executed |
| 9 | One Insert catalogue | `openInsertMenu` in `frontend/writer.js`; legacy controls hidden | Real browser asserts exactly one Table/Figure/Code/Inline/Display/Symbols/Publications entry | PASS | None found |
| 10 | Table caption above | `buildTable` emits caption/label before `tabular`; longtable retains valid top caption | Frontend unit test asserts ordering | PARTIAL | Generated table not compiled in browser |
| 11 | Figure caption below | `buildFigure` retains include then caption | Frontend unit test asserts figure source | PARTIAL | Uploaded image figure not compiled/visually inspected |
| 12 | No duplicated Insert commands in ellipsis | `commands()` exposes only one `Insert…` bridge, not individual insert functions | Real browser asserts individual insert actions absent from More actions | PASS | None found |
| 13 | Mentor PDF wider than source | `.mentor-shell .three-pane-workspace` role-only grid in `static/shells.css` | Real browser compares rendered source/PDF bounding boxes | PASS | No draggable pane resizer exists to persist a custom split |
| 14 | Annotation opens exact file/range or truthful fallback | Stable file ID + relative anchor resolution and reviewed-PDF fallback in `focusWriterReview` | Existing server review test covers stable review identity/privacy; second-file repeated-phrase browser case not run | PARTIAL | Run the specified second-file repeated-text browser case |
| 15 | Chapter destination | Wrapper/main-aware `insertionDirectories` and confirmed complete destination | Frontend unit test resolves `Thesis/chapters/chapter9.tex` | PARTIAL | Browser creation not executed |
| 16 | Image/asset destination | Same resolver prefers real `images/`/`assets/`; server still rejects traversal/cross-report IDs | Frontend unit test resolves wrapper `Thesis/images/plot.png`; server asset tests exist but were not rerun | PARTIAL | Browser upload not executed |
| 17 | Categorized publications | Typed publication metadata/API and guarded categorized `\bibitem` generator | DB contract persists all three statuses; frontend unit test validates grouping/duplicate-key rejection | PARTIAL | Publication UI and M7 bibliography output not browser-tested |
| 18 | Table-cell sizing | Builder exposes bounded column widths/minimum row height with geometry explanation | Frontend unit test validates nonuniform width and height | PARTIAL | Browser preview/compile with multiline cell not executed |
| 19 | PDF reading position and source location | Per-report page/zoom/offset persistence and exact-build SyncTeX action in `frontend/writer.js` | Existing M7 browser proves PDF refresh; position and source-location actions were not exercised | PARTIAL | Run both required browser assertions |
| 20 | Search in header | Search markup moved to header; explorer has no duplicate | Real browser asserts header placement and explorer absence | PASS | None found |
| 21 | Centred login logo | `.login-brand` flex centering, contained responsive image | Real browser checks computed `justify-content:center` and `object-fit:contain` before login | PASS | Custom uploaded-logo visual snapshot not retained |
| 22 | Three template arrangements | Migration 0028, persistence/Admin API and UI explicitly store `REPORT_CONTENT_ONLY`, `SEPARATE_FILES`, `SINGLE_SOURCE` | Rust/frontend compile and tests cover classification; no three-template browser journey | PARTIAL | Safe verified macro binding into arbitrary single-source copies is intentionally not implemented; currently warns and remains metadata-only |
| 23 | All requested project metadata | Typed table/DTOs in migration 0028 and `front_matter.rs`; browser editor and integration projection | Focused DB/API contract checks executive summary distinction, type, datasets, literal code, publications, canonical people/provenance, managed TeX and zero builds | PASS | Department/school display names remain `null` where VCAP stores only identities |
| 24 | Accessible symbol grid | Existing catalogue rendered as searchable `.symbol-grid` with names/tooltips/keyboard handling | Real browser asserts grid size, accessible name, tooltip and arrow navigation | PARTIAL | Browser insertion at a preserved selection was not executed; Rice reference PDF contents were not reviewed in this environment |

## Qualification results

- Browser/M7: `legacy_front_matter_institutional_api_contract` — **1 passed, 0 failed, 47 filtered**; output reported Leader/Writer/Mentor passed, review turn-taking passed, 27 populated PDF assertions, two manual compiles, and zero browser errors.
- Review lifecycle: `s5_review_authorization_lifecycle_suggestions_rounds_and_isolation` — **1 passed, 0 failed, 47 filtered**.
- Review lock/exact state: `manual_build_review_lock_and_integration_reads_share_exact_state` — **1 passed, 0 failed, 6 filtered**.
- Institutional credentials/scopes/read-only routes: `institutional_api_admin_lifecycle_scope_and_read_only_routes` — **1 passed, 0 failed, 47 filtered**.
- Realtime collaboration fixture: `realtime_collaboration_converges_persists_recovers_and_enforces_access` — **1 passed, 0 failed, 47 filtered** against its own disposable database.
- Frontend: `npm test` — **11 test files passed**.
- Source gates: `cargo test --workspace`, `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `git diff --check`, OpenAPI semantic checks, and `docker compose ... config --quiet` — **passed**.
- The full aggregate server database suite was not rerun after making the realtime fixture self-contained; the focused database tests above used separate, uniquely named disposable databases.

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
