# Mentor guide

Mentors use `/review`. Only Team Papers explicitly assigned to the signed-in Mentor are visible.

## Review workspace

- Source is read-only.
- The PDF.js viewer shows the current or last-good PDF.
- Linked source/PDF anchors, threads, suggestions, and review rounds preserve review context.
- A Mentor cannot mutate source through HTTP, WebSocket, or suggestion bypasses.

Submitted suggestions become source changes only when an authorized Writer accepts them. Archived papers remain available for historical reading but do not accept new review actions.

## Restoration requests

The **Restoration Requests** section lists requests for assigned Team Papers. Inspect the target immutable version and the Writer’s reason, then add an optional note and choose **Endorse** or **Reject**.

Endorsement moves a request to Admin review. Rejection closes it as `MENTOR_REJECTED`. A Mentor cannot alter the selected version or apply a restoration.
