# ADR-005: Versioned Compile

## Status

Superseded in September 2026 by manual-only compilation. The immutable build/version model remains accepted.

## Decision

Source collaboration is real-time. PDF production begins only after an authorized explicit Compile request. Saves, collaboration, metadata, restoration, review submission, page loads, and background work do not submit compilation.

Duplicate manual clicks for the same immutable state coalesce or reuse verified output, and queue admission remains bounded. Editing while H1 compiles may make H1 historical/stale, but does not enqueue H2 automatically.

Before manifest capture, a compile barrier flushes collaboration persistence. The immutable compile manifest conceptually includes:

- paper ID and document epoch;
- cutoff sequence and manifest revision;
- file state vectors or hashes;
- main file ID;
- pinned template version and file-policy version;
- state hash;
- authenticated requester; and
- the manual trigger and authenticated requester.

The TeX worker uses TeX Live and `latexmk` behind the `Sandbox` abstraction. Compilation never occurs in the API, parser, or collaboration process. Output consists of immutable PDF, log, and SyncTeX artifacts tied to the manifest.

## Stale artifacts

If H1 finishes while source is H2, H1 is retained historically but is not represented as current. The UI says the PDF is out of date until an authorized user manually compiles H2. A failed build also does not displace the last-good historical PDF.

Undo while a compile runs may make the running artifact historical/stale. Undo, save, parsing, Front Matter refresh, and restoration never trigger another build.
