# C4 V2 Authentication and Product Shells

Checkpoint C4 establishes the transitional authenticated-principal boundary and separate browser shells. It performs no data migration and adds no SQL migration.

## Transitional principal resolution

Every authenticated request resolves an opaque session to the canonical, enabled user and reads `global_user_roles` from PostgreSQL at request time.

- A role row resolves the identity as a V2 principal with exactly one `writer`, `mentor`, or `admin` role.
- No role row resolves the identity as a Legacy principal with its existing `student`, `professor`, or `admin` account type.
- When a V2 role exists, it is authoritative. The legacy account type and all legacy project, Team, group-manager, Writer, Mentor, and Project Manager grants contribute no authority.

Thus a legacy Admin assigned V2 Writer is only a Writer, and a legacy Professor assigned V2 Mentor is only a Mentor. C4 does not infer or assign a role for any unassigned user.

## Server-controlled browser login

The login page remains a native `<form method="post" action="/login">` with native email and password fields. The server validates the password, creates an opaque HttpOnly session cookie, and returns a `303 See Other`. Browser JavaScript does not submit credentials or own authentication. Logout remains native `POST /logout`, revokes the session, expires browser cookies, and redirects to `/`.

Successful login and authenticated root routing use this table:

| Principal | Login destination | `GET /` |
| --- | --- | --- |
| V2 Writer | `/write` | `303 /write` |
| V2 Mentor | `/review` | `303 /review` |
| V2 Admin | `/admin` | `303 /admin` |
| Legacy Admin | `/admin` | `303 /admin` |
| Legacy Student or Professor | `/` | Existing V1 workspace response |

Unauthenticated browser shell requests continue to receive the existing server-rendered login page. Invalid native login remains a `401` login response.

## Browser route authorization

| Route | V2 Writer | V2 Mentor | V2 Admin | Legacy Admin | Other Legacy |
| --- | --- | --- | --- | --- | --- |
| `/write` | Allow | `403` | `403` | `403` | `403` |
| `/review` | `403` | Allow | `403` | `403` | `403` |
| `/admin` | `403` | `403` | Allow | Allow during transition | `403` |
| `/workspace` | `403` | `403` | `403` | Existing V1 behavior | Existing V1 behavior |

V2 Admin therefore has no V1 writing workspace. The transitional Legacy Admin exception applies only when `global_user_roles` has no row for that user.

## API authorization boundary

The existing legacy workspace API authentication seam now rejects every V2 principal before V1 resource or project-role authorization runs. This covers project, source/file, compile, job, Team, membership, private-draft/publish, Research Group, and template-instantiation paths. V2 Writer, Mentor, and Admin therefore cannot inherit V1 mutation capability from retained legacy records. Dedicated V2 mutation APIs remain deferred.

Existing `/api/admin` operations allow V2 Admin and unassigned Legacy Admin only. V2 Writer, V2 Mentor, and legacy non-admin principals receive `403`. These operations remain control-plane operations and expose no source mutation.

`GET /api/v2/me` is the only new V2 API. It returns the authenticated V2 user's identifier, email, and exclusive role. Legacy principals receive `403`; unauthenticated requests receive `401`.

## Session invalidation and request-time defense

Successful initial role assignment, role change, and role removal delete all existing sessions for the affected user inside the same PostgreSQL transaction as the role mutation. Ownership and Paper Team invariant checks run before either role or session state changes, so a rejected transition preserves both.

Session lookup checks expiry and current account enablement and joins the current V2 role on every request. Deleted or revoked sessions cannot authorize, disabled accounts cannot authorize, and a stale client-side role value cannot retain capability. This request-time lookup remains defensive even though supported role operations revoke sessions.

## Product shells

The Writer shell at `/write` establishes **MY PAPERS**, **TEAM PAPERS**, **FILES**, **EDITOR**, **PDF**, **PROBLEMS**, **REVIEWS**, and **HISTORY**. It contains truthful foundation and empty states only. C4 provides no source editor, paper creation, team management, role management, Research Groups, or legacy Publish workflow.

The Mentor shell at `/review` establishes **ASSIGNED REVIEWS**, **ACTIVITY**, **REVIEW ROUNDS**, **READ-ONLY SOURCE**, **PDF**, **REVIEW THREADS**, **APPROVALS**, and **VERSIONS**. It is a distinct read-only review product rather than a disabled Writer interface. It has no source textarea or source/file mutation controls. Review persistence and annotation behavior are not implemented.

The Admin shell at `/admin` preserves the safe existing operational overview, users, legacy Teams, Research Groups, projects, templates, build queue, audit, and system endpoints. It visibly reserves Paper Team provisioning for C5. Its HTML contains no source editor and V2 Admin receives no Workspace, Writer, or Mentor navigation. A Legacy Admin receives a transitional Legacy Workspace link.

All three shells share the same light-only typography, spacing, border, button, focus, contrast, landmark, and responsive layout rules. Controls use semantic links, buttons, forms, headings, navigation, and live status regions.

## Migration safety

C4 does not populate `global_user_roles`, convert Student to Writer, convert Professor to Mentor, resolve C2 decisions, delete legacy roles or product records, or change migration `0012_v2_domain_foundation.sql`. Production V2 role count remains zero until an explicitly authorized later checkpoint provisions roles.

## Deferred work

- C5: controlled Admin V2 user and Paper Team provisioning, role-change workflows, and template versions.
- C6 and C7: dedicated V2 collaboration persistence, protocol, and Writer editor integration.
- Later review checkpoints: review rounds, threads, approvals, PDF annotations, and version workflows.
- Later migration checkpoints: reviewed conversion of legacy roles, projects, Teams, Research Groups, and private working state.
