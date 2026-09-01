# Admin guide

Admins use `/admin` as the V2 governance control plane. Admin inspection does not add the Admin as a paper member and does not permit joining a collaborative source-editing room.

## Control-plane sections

- **OVERVIEW** — product and queue summary.
- **V2 USERS** — provision users and assign one exclusive Writer, Mentor, or Admin role.
- **PAPER TEAMS** — create teams, assign Writers/Mentors, select or change the Writer Leader, and apply legal lifecycle transitions.
- **TEMPLATES** — inspect the existing immutable library and select a template at Team creation.
- **FILE POLICIES** — inspect stable file IDs and set server-enforced policies.
- **VERSIONS** — inspect immutable Team Paper history.
- **REVIEWS**, **BUILD QUEUE**, **AUDIT**, and **SYSTEM** — operational inspection using existing bounded APIs.

## Templates and policies

Selecting a template during Paper Team creation clones its immutable blobs into a new workspace, registers stable files, sets the declared main file, and records a truthful template identity hash. Template default policy is applied to every cloned file. With no template, the normal `main.tex` bootstrap is used.

Silent template updates are not supported. Applying or changing a template on an existing Team is explicitly unavailable in this RC because conflict-safe preview/update semantics are not yet shipped.

Policies are `EDITABLE`, `CONTENT_READ_ONLY`, `STRUCTURE_LOCKED`, `TEMPLATE_MANAGED`, and `HIDDEN_SYSTEM`. Changes take effect for open collaboration rooms; rejected edits return a policy/reload error rather than being silently discarded.

## Lifecycle and Team leadership

An Active Team may be frozen, submitted, or archived. A Frozen Team can return to Active or be archived. Submitted may be archived. Archived is terminal and retained rather than deleted. Frozen and Archived papers deny source and structural mutation.

Team creation requires exactly one Leader selected from the assigned Writers. Reassignment is transactional and cannot select a Mentor, Admin, or unassigned Writer or leave an active Team leaderless. Admin may inspect historical governance data under Audit, but ordinary Team revert decisions belong to the Team Leader.

The Writer toolbar exposes review/checkpoint/revert controls only to that selected Leader. Admin does not open review rounds or approve ordinary Team reverts; it manages membership and leadership from Paper Teams and may inspect retained history under Audit.

## Operator boundary

SYSTEM health is read-only. BUILD QUEUE uses only existing allow-listed actions. Host-level operational actions remain CLI-only in this release candidate. The API has no Docker-socket mount and no arbitrary shell endpoint.
