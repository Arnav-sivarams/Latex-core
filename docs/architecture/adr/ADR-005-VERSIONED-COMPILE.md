# ADR-005: Versioned Compile

## Status

Accepted for V2.

## Decision

Source collaboration is real-time. PDF production is a coalesced automatic background compile, initially triggered after approximately two seconds of inactivity.

Per Paper Team, at most one automatic build is running and at most one newer automatic build is pending. The pending slot always represents the latest state. If H1 is compiling while edits produce H2, H3, and H4, H1 remains running and H4 replaces the pending candidate; H2 and H3 are not individually queued.

Before manifest capture, a compile barrier flushes collaboration persistence. The immutable compile manifest conceptually includes:

- paper ID and document epoch;
- cutoff sequence and manifest revision;
- file state vectors or hashes;
- main file ID;
- pinned template version and file-policy version;
- state hash;
- authenticated requester; and
- trigger, such as automatic, manual, checkpoint, or submission.

The TeX worker uses TeX Live and `latexmk` behind the `Sandbox` abstraction. Compilation never occurs in the API, parser, or collaboration process. Output consists of immutable PDF, log, and SyncTeX artifacts tied to the manifest.

## Stale artifacts

If H1 finishes while source is H2, H1 is retained historically but is not promoted as current. H2 is scheduled or used, and the latest valid current PDF remains visible until a newer exact-state build succeeds. A failed build also does not displace the latest valid PDF.

Undo during debounce cancels or recomputes an obsolete candidate. Undo while compile runs may make the running artifact historical/stale; the pending slot becomes the latest state.

Saving and parsing never directly trigger compilation. Automatic compilation is driven by the separate scheduler after durable collaborative change and debounce.
