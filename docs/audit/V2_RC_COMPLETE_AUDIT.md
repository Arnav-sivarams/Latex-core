# LaTeX Core V2 RC complete audit

Audit date: 2026-08-31

Required source: `simplified-paper-platform-v2` at `0b8f8f6`

This is the live release-candidate audit matrix and defect ledger. `PENDING` is
used only while the audit is running. Before qualification, every matrix row
must be changed to `PASS`, `FAIL`, `BLOCKED`, or `NOT_APPLICABLE` and include
evidence. A PASS requires an exercised product flow, not static inspection.

Adoption note: this document was adopted as a pre-existing untracked audit
artifact for the current audit run. Its structure and test IDs were preserved;
pre-existing claims that had not yet been verified in this run were reset.

## Safety baseline

The following preflight counts were present in the adopted artifact. They are
retained as historical factual notes but are `UNKNOWN` until independently
re-verified in the current run.

| Entity | Preflight count |
|---|---:|
| users | 4 |
| global roles | 4 |
| personal papers | 0 |
| Paper Teams | 1 |
| memberships | 3 |
| files | 1 |
| review threads | 0 |
| versions | 2 |

Preflight repository result: NOT_RUN. Active `./latex-core status` and
`./latex-core doctor`: NOT_RUN. Previously recorded M7 digest (current-run
verification pending):
`sha256:8db804f76b8e80e5be9fb28ba14b0938df5989b7a8250ca6b0e9f3c200c4ee38`.

## Audit matrix

| ID | Subsystem | User role | Feature | Expected | Result | Severity | Defect | Repair commit/status |
|---|---|---|---|---|---|---|---|---|
| PRE-001 | Preflight | Operator | repository identity | required clean branch and HEAD | NOT_RUN | P0 | — | current-run verification pending |
| PRE-002 | Preflight | Operator | active health | status and doctor healthy | NOT_RUN | P1 | — | current-run verification pending |
| PRE-003 | Preflight | Operator | active counts | eight entity counts recorded read-only | NOT_RUN | P0 | — | current-run verification pending |
| ENV-001 | UAT | Operator | isolated database/blob root | unique disposable resources, never active data | PENDING | P0 | — | — |
| ENV-002 | UAT | Operator | exact four-role seed | product password/account path | PENDING | P1 | — | — |
| ENV-003 | UAT | Admin/Writer | seed Team and personal paper | created through product paths | PENDING | P1 | — | — |
| BROWSER-001 | Browser | All | real browser runtime | actual Playwright/browser execution | PENDING | P1 | — | — |
| AUTH-001 | Authentication | Admin | login and routing | `/admin`; Writer/Mentor/workspace denied | PENDING | P0 | — | — |
| AUTH-002 | Authentication | Mentor | login and routing | `/review`; Writer/Admin/workspace denied | PENDING | P0 | — | — |
| AUTH-003 | Authentication | Writer | login and routing | `/write`; Mentor/Admin/workspace denied | PENDING | P0 | — | — |
| AUTH-004 | Authentication | Anonymous | protected routes | login boundary, no protected data | PENDING | P0 | — | — |
| AUTH-005 | Authentication | All | logout/back/refresh | invalid session cannot recover privilege | PENDING | P0 | — | — |
| AUTH-006 | Authentication | All | V1 authority absence | no V1/group/professor/student/PM fallthrough | PENDING | P0 | — | — |
| EMPTY-001 | Empty states | Writer | no papers | truthful usable empty state | PENDING | P2 | — | — |
| EMPTY-002 | Empty states | Mentor | no assigned Teams | `No assigned Paper Teams`, no persistence failure | PENDING | P1 | DEFECT-A | — |
| EMPTY-003 | Empty states | Admin | no Teams | truthful usable empty state | PENDING | P2 | — | — |
| EMPTY-004 | Empty states | All | reviews/versions/PDF/build/restores/problems absent | no 500/spinner/blank/JS error | PENDING | P2 | — | — |
| USER-001 | Admin users | Admin | list/create Writer/Mentor/Admin | authoritative mutually exclusive roles | PENDING | P1 | — | — |
| USER-002 | Admin users | Admin | invalid/duplicate user input | clear validation and no duplicate | PENDING | P2 | — | — |
| USER-003 | Admin users | Admin | role change and session revocation | current authority changes immediately | PENDING | P0 | — | — |
| USER-004 | Admin users | Admin | enable/disable | supported controls revoke session | PENDING | P0 | — | — |
| CLI-001 | Operator CLI | Operator | `user list` role display | `v2=ROLE legacy=type`, including UNASSIGNED | PENDING | P2 | DEFECT-F | — |
| TEAM-001 | Provisioning | Admin | create Team and assignments | one workspace/Team/main.tex/Main/stable ID | PENDING | P1 | — | — |
| TEAM-002 | Provisioning | All | invalid membership/creation | Admin/wrong/duplicate rejected; non-Admin cannot create | PENDING | P0 | — | — |
| TPL-001 | Templates | Admin | local ZIP validate/preview | browser bytes; tree and Main selection | PENDING | P2 | DEFECT-D | — |
| TPL-002 | Templates | Admin | archive validation | traversal/absolute/duplicate/symlink/unsupported/empty/no-tex rejected | PENDING | P0 | DEFECT-D | — |
| TPL-003 | Templates | Admin | Main detection | obvious main auto-selected; ambiguity requires choice | PENDING | P2 | DEFECT-D | — |
| TPL-004 | Templates | Admin | immutable save | listed identity/hash with normal nested assets | PENDING | P2 | DEFECT-D | — |
| TPL-005 | Templates | Admin | Team from imported template | cloned stable files/Main/template pin | PENDING | P1 | DEFECT-D | — |
| TPL-006 | Templates | Admin | existing-Team update | explicitly unavailable in RC | PENDING | P3 | — | — |
| LIFE-001 | Lifecycle | Admin/Writer/Mentor | active/frozen/reactivated | mutations obey state | PENDING | P0 | — | — |
| LIFE-002 | Lifecycle | Admin/Writer/Mentor | submitted/archived | immutable while history remains | PENDING | P0 | — | — |
| LIFE-003 | Lifecycle | Admin | invalid transition | clear 409/validation | PENDING | P2 | — | — |
| DISC-001 | Discovery | Writer | own personal and assigned Team | both visible only to owner/member | PENDING | P0 | — | — |
| DISC-002 | Discovery | Writer/Mentor | personal-paper isolation | wrong Writer/Mentor denied on known ID | PENDING | P0 | — | — |
| FILE-001 | Files | Writer | create/nested/rename/move/delete | correct tree and tombstone behavior | PENDING | P1 | — | — |
| FILE-002 | Files | Writer | stable identity | ID survives rename/move/undo | PENDING | P1 | — | — |
| FILE-003 | Files | Writer | Set Main | exactly one valid Main | PENDING | P1 | — | — |
| FILE-004 | Files | Writer | invalid paths/duplicates | traversal/absolute/empty/duplicate rejected | PENDING | P0 | — | — |
| ASSET-001 | Assets | Writer | PNG/JPEG/PDF/CSV upload | correct binary behavior; raster preview | PENDING | P2 | — | — |
| ASSET-002 | Assets | Writer | invalid MIME/path/size/SVG | clear safe rejection | PENDING | P0 | — | — |
| COLLAB-001 | Collaboration | Writers | two-context convergence | bidirectional live edits and exact durable state | PENDING | P1 | — | — |
| COLLAB-002 | Collaboration | Writers | insert/delete/paste/Unicode | converges without full replacement/PUT | PENDING | P1 | — | — |
| UNDO-001 | Text undo | Writer 1 | scoped undo/redo | own AAA toggles, BBB preserved | PENDING | P1 | — | — |
| UNDO-002 | Text undo | Writer 2 | scoped undo/redo | own BBB toggles, AAA preserved | PENDING | P1 | — | — |
| OFFLINE-001 | Offline | Writer | status sequence | Offline local, Reconnecting, Syncing, Synced | PENDING | P2 | — | — |
| OFFLINE-002 | Offline | Writer | CRDT merge/reload recovery | no duplicate/loss; IndexedDB recovery | PENDING | P1 | — | — |
| TABS-001 | Browser storage | Writer | same-user tabs | convergence; closing one is harmless | PENDING | P1 | — | — |
| TABS-002 | Browser storage | Writers | logout/login isolation | recovery never crosses users; full identity key | PENDING | P0 | — | — |
| RESTART-001 | Recovery | Writers | API restart | exact source reconstruction and reconnect | PENDING | P1 | — | — |
| STRUCT-001 | Structural undo | Writer | create/rename/move/delete/Main undo-redo | state and IDs correct, blobs retained | PENDING | P1 | — | — |
| STRUCT-002 | Structural undo | Writers | cross-writer conflict | unsafe undo returns 409 without clobber | PENDING | P0 | — | — |
| POLICY-001 | Policies | Writer | EDITABLE | content and structure allowed | PENDING | P1 | — | — |
| POLICY-002 | Policies | Writer | CONTENT_READ_ONLY | content mutation denied | PENDING | P0 | — | — |
| POLICY-003 | Policies | Writer | STRUCTURE_LOCKED | content allowed; structure denied | PENDING | P0 | — | — |
| POLICY-004 | Policies | Writer | TEMPLATE_MANAGED | content and structure denied | PENDING | P0 | — | — |
| POLICY-005 | Policies | Writer/Mentor | HIDDEN_SYSTEM | hidden and direct fetch denied | PENDING | P0 | — | — |
| POLICY-006 | Policies | All | enforcement surfaces | HTTP/WS/undo/Main/suggestion obey policy | PENDING | P0 | — | — |
| POLICY-007 | Policies | Admin/Writer | live policy change | open socket's next mutation rejected/reloaded | PENDING | P0 | — | — |
| BUILD-001 | Auto build | Writer | two-second durable debounce | no early/per-keystroke build; local/remote reset | PENDING | P2 | — | — |
| COMPILE-001 | Manual compile | Writer | button and shortcut | valid exact state yields PDF | PENDING | P1 | DEFECT-B | — |
| COMPILE-002 | Manual compile | Mentor | assigned Team button | valid exact state yields PDF, no edit authority | PENDING | P1 | DEFECT-B | — |
| PIPE-001 | Compile pipeline | Writer/Mentor | exact barrier/queue/M7/artifacts | PDF/log/SyncTeX become Current | PENDING | P1 | DEFECT-B | — |
| PIPE-002 | Compile queue | Writers | dedup/coalescing | identical dedup; one active/newest pending | PENDING | P1 | — | — |
| PIPE-003 | Compile queue | Writers | H1–H4 rapid updates | bounded coalescing to newest | PENDING | P1 | — | — |
| PIPE-004 | Compile result | Writer | failure after last good | H1 PDF remains; H2 diagnostics; H3 promotes | PENDING | P1 | — | — |
| PIPE-005 | Compile result | Writer | stale promotion | late H1 historical; H2 current | PENDING | P0 | — | — |
| VERSION-001 | Versions | Writer | Alpha/Beta checkpoints/reload | immutable append-only persistence | PENDING | P1 | — | — |
| VERSION-002 | Versions | Writer | comparison | file and text diffs accurate | PENDING | P2 | — | — |
| PROD-001 | Productivity | Writer | outline/search/Quick Open | correct stable navigation and keyboard flow | PENDING | P2 | — | — |
| PROD-002 | Productivity | Writer | command palette/Problems | usable keyboard/dialog behavior | PENDING | P2 | — | — |
| PROD-003 | Productivity | Writer | PDF navigation/symbols/snippets | truthful navigation and insertion | PENDING | P2 | — | — |
| COMPLETE-001 | Autocomplete | Writer | commands | sec/sub/frac/begin/text insert correctly | PENDING | P2 | — | — |
| COMPLETE-002 | Autocomplete | Writer | bib/label completion | only real citations/references; no duplicate syntax | PENDING | P2 | — | — |
| PROBLEM-001 | Problems | Writer | reference/citation/syntax diagnostics | supported diagnostics and correct navigation | PENDING | P2 | — | — |
| BUILDER-001 | Builders | Writer | table/figure/equation/aligned | sane preview/source and truthful packages | PENDING | P2 | — | — |
| BUILDER-002 | Builders | Writer | matrix/cases/pgfplots | sane preview/source and truthful packages | PENDING | P2 | — | — |
| BUILDER-003 | Builders | Writer | algorithm/code/bibliography/theorem | sane preview/source and truthful packages | PENDING | P2 | — | — |
| BUILDER-004 | Builders | Writer | transaction/security | Writer Yjs insertion; no source-write endpoint/preamble mutation | PENDING | P0 | — | — |
| BUILDER-005 | Builders | Writer | representative compile | table/equation/matrix/figure compile | PENDING | P1 | — | — |
| MENTOR-001 | Mentor discovery | Mentor | remove/add assignment | Team disappears/returns; empty state truthful | PENDING | P1 | DEFECT-A | — |
| MENTOR-002 | Mentor source | Mentor | live read-only view | receives edits; typing/paste/delete impossible | PENDING | P0 | — | — |
| MENTOR-003 | Mentor source | Mentor | direct mutation attempts | REST and WS SOURCE_UPDATE rejected | PENDING | P0 | — | — |
| PDFJS-001 | PDF.js | Mentor | local viewer/worker controls | load/page/zoom/resize, no CDN | PENDING | P2 | — | — |
| PDFJS-002 | PDF.js | Mentor | overlay rerender | normalized overlay survives changes | PENDING | P2 | — | — |
| ROUND-001 | Review round | Mentor | open exact baseline once | exactly one OPEN persisted/rendered baseline | PENDING | P1 | DEFECT-C | — |
| ROUND-002 | Review round | Mentor | duplicate open/reload | coherent idempotent/conflict UX and reconstruction | PENDING | P2 | DEFECT-C | — |
| ANNO-001 | Annotation UX | Mentor | COMMENT/QUESTION/CHANGE_REQUEST | replacement hidden and inactive | PENDING | P2 | DEFECT-E | — |
| ANNO-002 | Annotation UX | Mentor | SUGGESTED_REPLACEMENT | replacement shown and required; dynamic switching | PENDING | P2 | DEFECT-E | — |
| THREAD-001 | Review types | Mentor | all six thread types | persisted, labeled, authorized | PENDING | P1 | — | — |
| META-001 | Review metadata | Mentor | four severities/categories/assignee/due | reload persistence | PENDING | P2 | — | — |
| META-002 | Review metadata | Mentor | wrong-team assignee | rejected | PENDING | P0 | — | — |
| STATE-001 | Review state | Mentor/Writer | OPEN→ADDRESSED→RESOLVED→REOPENED | role-authorized transitions | PENDING | P1 | — | — |
| STATE-002 | Review state | Mentor/Writer | invalid transitions/retries | 409; no duplicate messages | PENDING | P1 | — | — |
| ANCHOR-001 | Source anchors | Mentor/Writer | insertion drift/rewrite/delete | follows phrase then marks changed/deleted | PENDING | P1 | — | — |
| PDFANN-001 | PDF annotations | Mentor | rectangle persistence | normalized anchor survives zoom/resize/rerender | PENDING | P2 | — | — |
| SYNCTEX-001 | SyncTeX | Writer/Mentor | source↔PDF | exact/useful mapping where available | PENDING | P2 | — | — |
| SYNCTEX-002 | SyncTeX | Writer/Mentor | fallbacks | approximate/PDF-only, never fabricated | PENDING | P0 | — | — |
| SUGGEST-001 | Suggestions | Mentor/Writer | create/accept | Mentor does not edit; Writer CRDT accept and identity recorded | PENDING | P0 | — | — |
| SUGGEST-002 | Suggestions | Mentor/Writer | stale/reject | no blind mutation; source unchanged on reject | PENDING | P0 | — | — |
| APPROVAL-001 | Review approval | Mentor | blocking gate | unresolved blocks; resolved permits approval | PENDING | P1 | — | — |
| APPROVAL-002 | Review approval | Mentor/Writer | exact-state staleness/diff | real changes; old approval historical | PENDING | P1 | — | — |
| EXPORT-001 | Review export | Mentor | CSV | commas/quotes/newlines safely escaped | PENDING | P2 | — | — |
| EXPORT-002 | Review export | Mentor | print HTML | escaped and browser-loadable | PENDING | P0 | — | — |
| RESTORE-001 | Team restoration | Writer/Mentor/Admin | request/reject path | governed states and wrong-role denial | PENDING | P0 | — | — |
| RESTORE-002 | Team restoration | Writer/Mentor/Admin | endorse/apply path | only Admin applies endorsed request | PENDING | P0 | — | — |
| RESTORE-003 | Restoration safety | Admin | append-only cutover | H1 restored; H2 and safety/new-head history retained | PENDING | P0 | — | — |
| RESTORE-004 | Restoration safety | Clients | epoch cutover | increments once; reload; old updates rejected | PENDING | P0 | — | — |
| RESTORE-005 | Offline restore | Writer | old-epoch local edit | not merged; recovery buffer preserved/displayed | PENDING | P0 | — | — |
| RESTORE-006 | Personal restore | Writer | owner direct restore | append-only safety semantics | PENDING | P0 | — | — |
| RESTORE-007 | Personal restore | Writer | other owner probing | denied | PENDING | P0 | — | — |
| REVOKE-001 | Live revocation | Admin/Writer | remove Team member | open connection rejected/disconnected; discovery removed | PENDING | P0 | — | — |
| REVOKE-002 | Live revocation | Admin/User | disable/role change | existing session loses privilege and reroutes | PENDING | P0 | — | — |
| ADMINQ-001 | Build queue | Admin | status semantics | queued/running/succeeded/failed truthful | PENDING | P2 | — | — |
| ADMINQ-002 | Build queue | Admin | retry/cancel if exposed | authorized valid states; no source authority | PENDING | P0 | — | — |
| AUDIT-001 | Audit log | Admin | accountability events | who/what/when/target for covered actions | PENDING | P1 | — | — |
| AUDIT-002 | Audit log | Admin | secret hygiene | no passwords/hashes/tokens | PENDING | P0 | — | — |
| SYSTEM-001 | System | Admin | operational view | truthful read-only, no Docker/commands | PENDING | P0 | — | — |
| IDOR-001 | Authorization | Wrong principals | known object IDs | paper/thread/PDF/log/SyncTeX/version/restore denied | PENDING | P0 | — | — |
| XSS-001 | Output safety | All | hostile review/name text | rendered as text, no execution | PENDING | P0 | — | — |
| XSS-002 | Output safety | Mentor | CSV hostile text | correctly quoted | PENDING | P0 | — | — |
| IDEMP-001 | Idempotency | All | compile/checkpoint/thread/actions/restore | no unintended logical duplicates | PENDING | P1 | — | — |
| IDEMP-002 | Idempotency | Admin | Team double-submit | existing/validation/safe conflict | PENDING | P1 | — | — |
| SLOW-001 | Slow network | All | open/sync/build/review | pending states; no duplicate requests | PENDING | P2 | — | — |
| OUTAGE-001 | Recovery | Writer | API outage | local edit survives; reconnect; no loss | PENDING | P1 | — | — |
| OUTAGE-002 | Recovery | All | worker outage | build queues; editing/review work; restart completes | PENDING | P1 | — | — |
| MULTI-001 | Collaboration | Writers | two files simultaneous | barrier captures both newest durable states | PENDING | P1 | — | — |
| REMOTE-001 | Files | Writers | rename/delete open remote file | ID maintained, then deleted/reload state enforced | PENDING | P1 | — | — |
| MAIN-001 | Files | Writers | concurrent Set Main | serialized, exactly one authoritative Main | PENDING | P0 | — | — |
| LARGE-001 | Storage | Writers | large paste and multilingual UTF-8 | survives sync/reload/restart/checkpoint | PENDING | P1 | — | — |
| A11Y-001 | Accessibility | All | keyboard core flows | sane tab/focus/Escape, no traps | PENDING | P2 | — | — |
| VIEW-001 | Layout | All | 1366×768 and 1920×1080 | usable shells/tables/editor/PDF | PENDING | P2 | — | — |
| VIEW-002 | Layout | All | zoom 80/125/150% | usable with aligned overlays | PENDING | P2 | — | — |
| CONSOLE-001 | Browser diagnostics | All | successful flows | no unexplained exceptions/rejections/loops/404/500 | PENDING | P1 | — | — |
| STATIC-001 | Static assets | Anonymous | six required local assets | local HTTP 200 and no runtime CDN | PENDING | P1 | — | — |
| CONCUR-001 | Bounded concurrency | Writers | at least 12 WS clients | convergence/persistence; no panic/deadlock/loss | PENDING | P1 | — | — |
| MIGRATE-001 | Migrations | Operator | empty DB 001–017 | all apply; expected tables present | PENDING | P1 | — | — |
| C2-001 | Legacy rehearsal | Operator | frozen C0 planner twice | byte-equivalent; unresolved decisions remain | PENDING | P1 | — | — |
| LAUNCH-001 | Operator UX | Operator | start/status/doctor/user list | truthful and usable | PENDING | P1 | DEFECT-F | — |
| DOC-001 | Documentation | All | README/guides vs product | listed V2 behavior and limits accurate | PENDING | P2 | — | — |
| GATE-001 | Release gates | Developer | npm gates | clean install/build/tests pass | PENDING | P1 | — | — |
| GATE-002 | Release gates | Developer | Rust gates | fmt/check/test/Clippy pass | PENDING | P1 | — | — |
| GATE-003 | Release gates | Developer | DB/JS/diff checks | clean disposable DB, syntax, diff pass | PENDING | P1 | — | — |
| ACTIVE-001 | Safety | Operator | post-audit active counts | unchanged absent identified user action | PENDING | P0 | — | — |
| ACTIVE-002 | Safety | Operator | final active health/M7 | status/doctor healthy; M7 unchanged | PENDING | P0 | — | — |
| DEPLOY-001 | Deployment | Operator | affected service deployment | no reset/rebuild M7; routes/assets healthy | PENDING | P1 | — | — |
| RERUN-001 | Second UAT | All | brand-new clean complete rerun | no open P0/P1/P2 or fixture dependency | PENDING | P0 | — | — |

## Defect ledger

Known defects are listed before reproduction so they cannot be lost. Root cause,
repair files, and regression proof are filled only after evidence exists.

| Defect ID | Test ID | Severity | Reproduction | Root cause | Files changed | Fix | Regression test | Status |
|---|---|---|---|---|---|---|---|---|
| DEFECT-A | EMPTY-002, MENTOR-001 | P1 | User-observed Mentor `/review` persistence failure with no assigned papers; isolated reproduction pending | pending | pending | pending | pending | OPEN |
| DEFECT-B | COMPILE-001, COMPILE-002, PIPE-001 | P1 | User-observed valid minimal Mentor compile failure; Writer comparison pending | pending | pending | pending | pending | OPEN |
| DEFECT-C | ROUND-001, ROUND-002 | P2 | User-observed contradictory missing-baseline UI and duplicate-baseline toast; isolated reproduction pending | pending | pending | pending | pending | OPEN |
| DEFECT-D | TPL-001–TPL-005 | P2 | Admin browser local ZIP import is absent | pending | pending | pending | pending | OPEN |
| DEFECT-E | ANNO-001, ANNO-002 | P2 | Replacement remains visible for CHANGE_REQUEST | pending | pending | pending | pending | OPEN |
| DEFECT-F | CLI-001, LAUNCH-001 | P2 | `user list` exposes only legacy account type | pending | pending | pending | pending | OPEN |

## Evidence log

Evidence entries include the environment identifier, actual browser/API/SQL/test
command, observable result, and any expected negative HTTP status. Disposable
credentials and secrets are never committed.

| Timestamp UTC | Test IDs | Environment | Evidence/result |
|---|---|---|---|
| 2026-08-31 adopted artifact | PRE-001–PRE-003 | pre-existing record | Historical notes only; current-run verification reset to `NOT_RUN`/`UNKNOWN` |

## Remaining limitations

To be completed truthfully after the second UAT pass. Existing-Team template
update is an explicit RC limitation unless the audited product says otherwise.
