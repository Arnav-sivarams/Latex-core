# Mentor guide

Mentors use `/review`. Only Team reports explicitly assigned to the signed-in Mentor are visible.

## Review workspace

- Source is read-only.
- The PDF.js viewer shows the current or last-good PDF.
- Source and the latest successful PDF remain readable whether or not a review is open.
- Comment and Suggestion controls are enabled only during that Mentor's active participation after the Team Leader has sent an exact current build for review.
- Linked source/PDF anchors, threads, suggestions, and review rounds preserve review context.
- A Mentor cannot mutate source through HTTP, WebSocket, or suggestion bypasses.
- Project PNG/JPEG files can be opened as authenticated, read-only previews. Their stable file identity and bytes remain scoped to the assigned report; knowing another report's file ID or blob hash grants no access.

The normal Mentor viewport contains only the compact toolbar and independently scrolling Papers/Files, read-only source, and PDF panes. Before review opens, the toolbar quietly says **Waiting for Team Review**; source and PDF remain readable and selection behaves normally.

During an open review, select a non-empty source range or drag a PDF region, then right-click the selection to open the small review popover. It has one text input and Comment/Suggest actions—no severity, category, Writer assignment, or due-date form. Escape or clicking outside closes it without moving the page. The toolbar Comments action opens active/resolved history in a temporary drawer.

Creating, editing, or deleting feedback saves a private server-side draft. **Draft saved — not yet visible to writers** means it will survive reloads and restarts but is excluded from Writer/Admin APIs, live events, previews, and exports. The prominent **Push review** action confirms the saved draft count and publishes only the signed-in Mentor's current draft set. A failed or conflicting save is retained for retry and blocks submission rather than omitting feedback.

With multiple assigned Mentors, one Mentor's submission becomes visible immediately and completes only that Mentor's participation. The round stays open for the others and closes after the last required Mentor submits. A Leader withdrawal does not publish private drafts. Published unresolved feedback remains available after round closure.

Submitted suggestions become source changes only when an authorized Writer accepts them. New feedback uses a simple Comment or Suggestion body; severity, category, assignment, and due date are not requested. Archived papers remain available for historical reading but do not accept new review actions.

Mentors do not participate in Team revert authorization. Revert requests are handled within the Team by its Writer Leader.

**Editor settings** stores the signed-in Mentor's source-editor font size and Light/Dark editor theme without recreating the read-only Yjs document or altering shared source/PDF output.
