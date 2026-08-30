# ADR-010: Security Boundaries

## Status

Accepted for V2.

## Server-side invariants

1. A Mentor can never submit a source mutation.
2. An Admin can never submit a source mutation.
3. Only an Admin can create or modify Paper Teams.
4. A Writer can access owned personal papers and assigned team papers only.
5. A Mentor can access assigned team papers only.
6. Admin global inspection occurs through `/api/admin` only.
7. Admin inspection does not create Paper Team membership.
8. WebSocket authentication is checked at connect.
9. Membership, role, status, or policy changes invalidate affected active capability.
10. Removing a user from a Paper Team terminates or revokes the active paper session.
11. File policy is checked before accepting a source update.
12. Artifact reads remain authorization-scoped.
13. Mentor suggestions never become source edits unless a Writer accepts.
14. An accepted source mutation carries authenticated Writer attribution.
15. Direct API or WebSocket calls cannot bypass template protection.
16. Restore requires a valid governance state and is revalidated at apply.
17. A global role change invalidates or revokes existing sessions.
18. The browser and API expose no arbitrary host-command execution.

## Process and trust boundaries

The API/control plane, collaboration gateway, and parser never execute user LaTeX or extensions. Compilation uses TeX Live and `latexmk` only in a worker behind the `Sandbox` abstraction. PostgreSQL is the source of truth for structured durable state and the queue. Blob reads and artifacts are checked against authenticated paper access, not merely possession of an identifier.

All mutation endpoints enforce role, resource access, current epoch/revision where applicable, and policy in the server. Client-shell separation improves clarity but is not the security boundary.
