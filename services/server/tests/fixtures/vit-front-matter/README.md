# Synthetic VIT Front Matter compatibility fixture

These four files are synthetic. They contain no uploaded archive or institutional data.

The tested compatibility contract is a ZIP containing recognized root-level sections:
`coverpage.tex` or `cover.tex`, `certificate.tex`, `declaration.tex`, and
`acknowledgement.tex` or `acknowledgements.tex`. A subset is accepted with warnings;
conflicting aliases are rejected. Section order is cover, certificate, declaration,
acknowledgement. Existing schema-v1 `frontmatter.json` packs retain their manifest
and placeholder renderer. Legacy imports receive an immutable internal schema-v2
manifest; imported TeX bytes remain unchanged.

The registry in `src/front_matter/legacy.rs` covers the VIT zero-argument macros in
this fixture. Scanning recognizes normal TeX control words and `%` comments,
including escaped percent signs and doubled backslashes. It does not interpret
arbitrary catcode changes, execute TeX conditionals, or support every third-party
template. Unmapped commands in the listed metadata families get empty definitions
and warnings; general LaTeX commands are not redefined. Missing mapped values use
`providecommand` only, retaining any existing template fallback. Resolved values
use the shared text escaper and override zero-argument placeholders safely.

AUTO sources are canonical Team names, ordered linked Students, assignment-group
semester, assigned Faculty Mentors, programme HOD, and unambiguous active Dean
roles scoped to the programme or its applicable School. Guide/Dean ambiguity
requires a constrained identity choice. Course, programme/degree/specialization,
School and Department display values remain manual where the schema has no exact
mapping/display value. One `submission_date` supplies month and year. Only manual
values and identity choices are stored as Team overrides in legacy mode.

The generated wrapper and `metadata.tex` use the existing HIDDEN_SYSTEM transaction,
BlobStore, workspace revisions, snapshots, and compile path. Reads create no history
event. Saves capture a prior exact state; the regression test restores matching
metadata bytes. A compatible main template must contain the existing integration
marker and load Front Matter in the document body.

Focused tests (use an empty disposable PostgreSQL database):

```sh
TEST_DATABASE_URL=... cargo test -p server --features database-tests --bin latex-core-api front_matter
```

The single opt-in browser journey uses the same authenticated fixtures and the
normal compilation worker, pinned to the frozen M7 image digest. It requires Docker,
Playwright Chromium and its runtime libraries; it does not run the M7 corpus:

```sh
FRONTMATTER_BROWSER=1 TEST_DATABASE_URL=... cargo test -p server --features database-tests --bin latex-core-api legacy_front_matter_institutional_api_contract -- --nocapture
```

`PLAYWRIGHT_CHROMIUM_PATH` may select an installed Chromium binary. All fixture
records have generated unique IDs; no existing database is reset. Browser output
reports the compiler environment identity, populated PDF assertions, optional-blank
compile, role checks, and captured errors.

## Validation recorded on 2026-09-13

- Frozen image: `latex-core-texlive@sha256:8db804f76b8e80e5be9fb28ba14b0938df5989b7a8250ca6b0e9f3c200c4ee38`.
- Compiler environment: `texlive-2026-sha256-364cea85dc8ba5e2a5131f1d8088142d08c9733d24d2644880d6eae86f751d5a`.
- Authenticated Leader, regular Writer and Mentor journey passed; 27 populated-PDF
  assertions passed. Omitting specialization compiled successfully, removed the
  value from PDF text, and displayed its warning. No captured browser errors or 5xx.
- Workspace tests: 139 passed. Focused Front Matter tests with database features:
  13 passed, including legacy and schema-v1 authorization/history regressions.
  Frontend tests: 11 passed.
- Formatting, workspace/all-target checking, focused production Clippy with warnings
  denied, JavaScript syntax checks, and whitespace checks passed. Broader Clippy
  also encountered existing warnings in unrelated persistence integration tests;
  those files were not changed.
