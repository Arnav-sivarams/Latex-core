# Mentor guide

Mentors use `/review`. Only Team Papers explicitly assigned to the signed-in Mentor are visible.

## Review workspace

- Source is read-only.
- The PDF.js viewer shows the current or last-good PDF.
- Source and the latest successful PDF remain readable whether or not a review is open.
- Comment and Suggestion controls are enabled only while the Team Leader has sent an exact current build for review.
- Linked source/PDF anchors, threads, suggestions, and review rounds preserve review context.
- A Mentor cannot mutate source through HTTP, WebSocket, or suggestion bypasses.

Submitted suggestions become source changes only when an authorized Writer accepts them. New feedback uses a simple Comment or Suggestion body; severity, category, assignment, and due date are not requested. Archived papers remain available for historical reading but do not accept new review actions.

Mentors do not participate in Team revert authorization. Revert requests are handled within the Team by its Writer Leader.
