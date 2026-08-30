# V2 Client Shells

## Routing contract

| Global role | Client route |
| --- | --- |
| Writer | `/write` |
| Mentor | `/review` |
| Admin | `/admin` |

Authentication resolves exactly one global role and routes to exactly one shell. Cross-shell access is rejected server-side.

## Writer shell

The Writer shell contains: **MY PAPERS**, **TEAM PAPERS**, **FILES**, **EDITOR**, **PDF**, **PROBLEMS**, **REVIEWS**, and **HISTORY**. It supports personal-paper ownership and assigned-team collaboration subject to membership and file policy.

## Mentor shell

The Mentor shell contains: **ASSIGNED REVIEWS**, **READ-ONLY SOURCE**, **PDF**, **REVIEW THREADS**, **ACTIVITY**, **VERSIONS**, and **APPROVALS**. Source is a live read-only projection with annotation tools. The Mentor does not receive a disabled Writer interface and cannot emit source or structural mutations.

## Admin shell

The Admin shell contains: **OVERVIEW**, **USERS**, **PAPER TEAMS**, **TEMPLATES**, **FILE POLICIES**, **VERSIONS**, **RESTORATION REQUESTS**, **REVIEWS**, **BUILD QUEUE**, **AUDIT**, and **SYSTEM**. It operates through control-plane APIs. The Admin does not receive a Writer interface or ordinary collaboration-room membership.

These are separate product shells, not feature flags applied to one editor shell.
