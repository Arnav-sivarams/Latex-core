# Admin guide

Admins use `/admin` as the V2 governance control plane. Admin inspection does not add the Admin as a paper member and does not permit joining a collaborative source-editing room.

## Control-plane sections

- **OVERVIEW** — product and queue summary.
- **V2 USERS** — provision users and assign one exclusive Writer, Mentor, or Admin role.
- **INSTITUTION DATA** — server-paginated Students, Faculty, Programmes, and identity-link state.
- **IMPORTS** — upload CSV/XLSX, validate, review bounded results, apply, and inspect paginated history.
- **PAPER TEAMS** — server-paginated Team grid, manual creation, unresolved imports, safe template override, and lifecycle actions.
- **PROGRAMME TEMPLATES** — programme mappings, global fallback, and ordered-Writer resolution preview.
- **TEMPLATES** — inspect the existing immutable library and select a template at Team creation.
- **FILE POLICIES** — inspect stable file IDs and set server-enforced policies.
- **VERSIONS** — inspect immutable Team Paper history.
- **REVIEWS**, **BUILD QUEUE**, **AUDIT**, and **SYSTEM** — operational inspection using existing bounded APIs.

## Templates and policies

Selecting a template during Paper Team creation clones its immutable blobs into a new workspace, registers stable files, sets the declared main file, and records a truthful template identity hash. Template default policy is applied to every cloned file. With no template, the normal `main.tex` bootstrap is used.

Existing-Team changes are a separate two-step operation. Preview compares the exact current workspace with the old and new immutable templates. Apply is refused if a Writer-created or Writer-modified file would be replaced, if Main changes without confirmation, if the Team is archived, or if the preview token is stale. A successful apply creates `PRE_TEMPLATE_CHANGE`, adds or updates only safe files, preserves all other files, changes the pin to `MANUAL_OVERRIDE`, and creates `TEMPLATE_UPDATE`.

Policies are `EDITABLE`, `CONTENT_READ_ONLY`, `STRUCTURE_LOCKED`, `TEMPLATE_MANAGED`, and `HIDDEN_SYSTEM`. Changes take effect for open collaboration rooms; rejected edits return a policy/reload error rather than being silently discarded.

## Lifecycle and Team leadership

An Active Team may be frozen, submitted, or archived. A Frozen Team can return to Active or be archived. Submitted may be archived. Archived is terminal and retained rather than deleted. Frozen and Archived papers deny source and structural mutation.

Team creation requires exactly one Leader selected from the assigned Writers. Reassignment is transactional and cannot select a Mentor, Admin, or unassigned Writer or leave an active Team leaderless. Admin may inspect historical governance data under Audit, but ordinary Team revert decisions belong to the Team Leader.

The Writer toolbar exposes review/checkpoint/revert controls only to that selected Leader. Admin does not open review rounds or approve ordinary Team reverts; it manages membership and leadership from Paper Teams and may inspect retained history under Audit.

## Institutional operations

Import modes are shown as Validate only, Merge / Update, and Import More — Add Only. Validation never applies data. Merge is additive: absent rows never remove records or Team memberships. Import More inserts only unseen keys. The result view limits displayed errors and provides the complete formula-neutralized `errors.csv` download.

Manual links require a compatible existing V2 role: Student to Writer and Faculty to Mentor. Admin identity linkage grants no role. Automatic reconciliation preserves `MANUAL` links. An identity used by a non-archived imported Team cannot be unlinked.

Manual Team creation searches accounts server-side, preserves Writer order, restricts Leader to those Writers, and previews programme resolution. An explicit template is recorded as `MANUAL_OVERRIDE`; otherwise MODE, TIE_FIRST_WRITER, or GLOBAL_FALLBACK provenance is retained.

## Operator boundary

SYSTEM health is read-only. BUILD QUEUE uses only existing allow-listed actions. Host-level operational actions remain CLI-only in this release candidate. The API has no Docker-socket mount and no arbitrary shell endpoint.
