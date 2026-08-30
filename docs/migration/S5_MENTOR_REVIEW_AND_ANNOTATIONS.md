# S5 Mentor review and linked annotations

S5 adds an additive review domain for assigned V2 Paper Teams. It does not alter S3 collaboration, S4 version/build ownership, the durable compilation queue, BlobStore, or frozen M7.

## Review schema

Migration `0015_v2_reviews.sql` adds review rounds, threads, chronological messages, source anchors, PDF anchor projections, and suggested replacements. Closed database values cover thread type/state, severity, category, suggestion state, round state, and mapping confidence. Review rows use restrictive foreign keys; source and artifact history is not rewritten.

## Authorization

Mentors can review only Paper Teams where `paper_team_members` assigns them. They may open/approve rounds, create and control threads, reply, resolve/reopen, approve sections/papers, and manually request the existing exact-state build. Writers assigned to the same Team may read/reply, mark addressed, and accept/reject suggestions. Admin is excluded from review conversation/mutation endpoints. Personal papers are not Mentor-reviewable in S5.

The Mentor WebSocket remains S3 `read_only`; server handling rejects source update frames. Mentor UI has no source-writing operation. An accepted suggestion is applied in the Writer browser as a normal Yjs transaction, flushed through the authenticated Writer WebSocket, and recorded as accepted only after the API verifies that exact durable collaboration sequence belongs to that Writer and file.

## Anchors and drift

Source anchors store browser-produced Yjs `RelativePosition` bytes, stable file UUID, quote, nearby-context SHA-256, document epoch, and source/version sequence. The UI resolves them against current Yjs state. Deleted files report `SOURCE_DELETED`; materially different resolved text reports `SOURCE_CHANGED`. Ordinary insertions before an anchor preserve its location.

PDF anchors store the exact S4 build/artifact, page, and normalized rectangles. PDF.js renders only the current page, with navigation/zoom and per-page overlays; normalized geometry survives zoom and browser size changes. Historical geometry remains append-only when the current build changes.

## SyncTeX and confidence

M7 contains the `synctex` CLI, but the API intentionally has no Docker socket and never executes Docker. S5 preserves that security boundary and uses a narrow, non-executing reader over the persisted S4 `.synctex.gz` artifact. Forward mapping selects the closest recorded box for source file/line; inverse mapping selects a recorded box for PDF page/point and resolves its path to a stable file UUID. Exact recorded line/box hits are `EXACT`; nearby records are `APPROXIMATE`; absent/invalid artifacts and unresolved paths are `PDF_ONLY`. No coordinate or source location is invented.

## Rounds, versions, and approvals

Opening a round captures the current successful S4 build version as its baseline. Approval is rejected while any `BLOCKING` thread is unresolved and creates an immutable S4 `review_round` version. Changes-since-review reuses the S4 manifest comparison. Section approvals retain their source anchor. Paper approvals store the exact version, build, and state hash; newer source therefore makes the approval historical instead of silently current.

Activity combines review records with existing compile/version rows. Reports are available as CSV and print-ready HTML for browser Print → Save as PDF; no bespoke PDF generator was added.

## Dependencies

PDF.js is pinned through local `pdfjs-dist`; runtime and worker are copied into same-origin static assets during `npm run build`. S6 can add Writer productivity and structured LaTeX builders without changing review attribution. S7 owns Team governance/restoration and must preserve review/version history.
