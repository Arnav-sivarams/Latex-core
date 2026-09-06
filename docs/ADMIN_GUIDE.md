# Admin guide

## Front Matter Packs and document metadata

The single **Templates** navigation item contains Main Content Templates, Front Matter Packs, and Automatic Defaults. Import a pack ZIP only after its preview confirms the ordered sections, fields, allowed automatic sources, entry file, and safe file tree. Pack content is immutable; only its name and description can be edited, and an in-use pack cannot be removed.

The Automatic Defaults table maps each programme independently to a Main Content Template and a Front Matter Pack. The global Front Matter fallback may be **None**. Defaults apply only to future Teams. Team **View / Manage** shows the current Front Matter status and permits an independent change or removal. Assignment is blocked when the pinned Main Content Template is reported as **Front Matter not enabled**. See [Front Matter Packs](FRONT_MATTER_PACKS.md).

Admins use `/admin` as the V2 governance control plane. Admin inspection does not add the Admin as a paper member and does not permit joining a collaborative source-editing room.

## Control-plane sections

The fixed Admin shell groups navigation into **People & data**, **Papers**, and **Operations**. These labels are visual aids rather than additional destinations; the navigation and current section scroll independently.

- **Overview** — product and queue summary, plus actionable items that need attention.
- **V2 Users** — manually provision users, inspect account and email-delivery state, and generate one-time temporary passwords for Writers or Mentors.
- **Institution Data** — server-paginated institutional datasets with manual Add, Edit, dependency-previewed Delete, and Paper Assignment membership management.
- **Imports** — multi-file CSV/XLSX drag/drop, automatic dataset detection, one batch review/apply action, and concise paginated history.
- **Paper Teams** — server-paginated Team grid, manual creation, full membership editing, unresolved imports, safe template changes, and lifecycle actions.
- **Templates** — Main templates, Front Matter, and Automatic defaults in one page.
- **File Policies** — inspect ordinary Team files and set server-enforced policies.
- **Versions** — inspect immutable Team Paper history; revert authority remains with the Writer Leader.
- **Reviews**, **Build Queue**, and **Audit** — read-only operational inspection using existing bounded APIs.
- **System** — read-only health plus application branding. An Admin can upload, preview, replace, or reset the shared header logo.

Branding accepts at most 512 KiB and decodes only PNG, JPEG, or WebP images with dimensions from 1×1 through 2048×2048. Filename extensions and submitted MIME types are not trusted. The approved blob is served from a fixed same-origin route; clients cannot provide filesystem paths or remote URLs. Branding affects application/login headers only and never changes Front Matter or existing PDFs.

## Templates and policies

Selecting a template during Paper Team creation clones its immutable blobs into a new workspace, registers stable files, sets the declared main file, and records a truthful template identity hash. Template default policy is applied to every cloned file. With no template, the normal `main.tex` bootstrap is used.

Existing-Team changes appear as one **Change template** action. Internally, the server still previews the exact current workspace and validates a short-lived state token before apply. The Admin sees a plain file-count confirmation or a list of conflicting Writer-edited paths; a Main-document checkbox appears only when Main changes. A successful apply still creates `PRE_TEMPLATE_CHANGE`, updates only safe files, preserves all other files/history, changes the pin to `MANUAL_OVERRIDE`, and creates `TEMPLATE_UPDATE`.

Policies are `EDITABLE`, `CONTENT_READ_ONLY`, `STRUCTURE_LOCKED`, `TEMPLATE_MANAGED`, and `HIDDEN_SYSTEM`. Changes take effect for open collaboration rooms; rejected edits return a policy/reload error rather than being silently discarded.

## Lifecycle and Team leadership

An Active Team may be frozen, submitted, or archived. A Frozen Team can return to Active or be archived. Submitted may be archived. Archived is terminal and retained rather than deleted. Frozen and Archived papers deny source and structural mutation.

Team creation and **Edit Team** require at least one ordered Writer and exactly one Leader selected from those Writers. Writers and Mentors come from server-side role-filtered account search. An imported Team edit transaction updates its institutional assignment metadata/members and canonical runtime membership together without recreating its Team, workspace, versions, comments, or builds.

The Writer toolbar exposes review/checkpoint/revert controls only to that selected Leader. Admin does not open review rounds or approve ordinary Team reverts; it manages membership and leadership from Paper Teams and may inspect retained history under Audit.

## Institutional operations

The primary operations are Add, Edit, and Delete. Add never changes an existing key. Edit changes only supplied fields on known keys and never inserts. Delete affects only explicitly supplied keys; omission is never synchronization-by-absence. Multi-file validation sees parents staged anywhere in the batch and Apply chooses dependency order automatically. The bounded review uses dataset names, record counts, actionable file/row issues, and field-level Edit diffs. IDs, checksums, file jobs, and formula-neutralized `errors.csv` downloads are under Technical details.

Manual forms use the same authoritative server validation. A manual Delete always checks dependencies first. Institutional deletion never removes a V2 account or paper history, and a materialized Paper Assignment is blocked pending explicit Team lifecycle handling. Legacy single-file import history is not part of the normal Admin interface.

During Add, each valid imported Student receives or reuses a V2 Writer account. Only Faculty referenced by `paper_team_mentors` receive or reuse a V2 Mentor account. Existing compatible accounts retain their password; incompatible roles are reported and never changed. `vcap.admins` never provisions or grants V2 Admin. Automatic reconciliation preserves `MANUAL` links, and an identity used by a non-archived imported Team cannot be unlinked.

New automatic accounts receive an eight-character temporary password and `must_change_password=true`. Apply returns those new credentials once; the browser immediately downloads `email,password,role` CSV and does not persist the plaintext. In that CSV, `role=student` means V2 `WRITER`. A temporary login can reach only **Set your password** until the user chooses a 12–256 character permanent password. Admin can generate a replacement temporary password once for an existing Writer or Mentor; current sessions are revoked and the old password is never shown.

Manual Team creation searches accounts server-side, preserves Writer order, restricts Leader to those Writers, and shows the resolved template contextually. An explicit template is recorded as `MANUAL_OVERRIDE`; otherwise MODE, TIE_FIRST_WRITER, or GLOBAL_FALLBACK provenance is retained. The diagnostic resolution endpoint remains available, but there is no permanent resolution-calculator panel.

## Operator boundary

SYSTEM health is read-only. BUILD QUEUE uses only existing allow-listed actions. Host-level operational actions remain CLI-only in this release candidate. The API has no Docker-socket mount and no arbitrary shell endpoint.

## Temporary credential delivery

New institutional Students and assigned Faculty receive Writer/Student and Mentor temporary
credentials by email. Existing compatible accounts are reused without a password change or email.
Generating a new temporary password revokes existing sessions and queues a fresh email; explicit
permanent passwords are never emailed.

The V2 Users table reports pending, sending, sent, failed, and expired delivery states. A failed
delivery can be retried while its encrypted payload remains valid. After expiry, generate a new
temporary password. Successful and expired jobs immediately discard their encrypted payload.

The one-time `credentials.csv` download remains available as an administrator fallback. Save it
when offered: temporary passwords cannot be viewed again.
