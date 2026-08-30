# ADR-007: Mentor Review

## Status

Accepted for V2.

## Feedback model

Review types are Comment, Question, Change Request, Suggested Replacement, Section Approval, and Paper Approval. Severity is `note`, `minor`, `major`, or `blocking`. The category set is `writing`, `methodology`, `evidence`, `citation`, `formatting`, `figure`, `table`, `equation`, and `submission_requirement`.

Threads move from `OPEN` to `ADDRESSED` when a Writer reports action, then to `RESOLVED` only when a Mentor accepts the resolution. A Mentor may move `ADDRESSED` or `RESOLVED` to `REOPENED`. A Writer cannot resolve Mentor feedback.

Mentors may attach feedback to CRDT-relative source ranges, PDF regions, or both; assign it to a Writer; set a due date where supported; and reply through append-preserving messages.

## Suggested replacement

A Mentor proposes replacement text but never mutates source. A Writer may accept it, edit it before accepting, or reject it with a reason. Acceptance creates an actual source CRDT transaction attributed to the authenticated Writer, with the suggestion recorded as provenance. A Mentor cannot accept their own suggestion.

## Approval

Section approval, review-round approval, and paper approval are explicit audited review records tied to the reviewed state. They do not silently grant source mutation, change membership, restore history, or perform administrative submission.
