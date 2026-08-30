# Current to V2 Component Migration Map

This map describes intended future disposition. “Retire” and “replace” are architecture decisions, not claims that C1 removed current code or data.

| Current component | Current purpose | V2 decision | V2 replacement | Migration checkpoint |
| --- | --- | --- | --- | --- |
| Native server-controlled auth and session cookies | Credential verification, enablement, sessions, and account-type gates | Keep foundation; replace role vocabulary and revocation semantics | Native auth with exclusive `users.role`, shell routing, and role/session invalidation | C2 planning; later auth checkpoint |
| PostgreSQL | Structured state, durable queue, events, snapshots, auth, collaboration records | Keep and evolve | V2 source of truth, collaboration log/snapshots, review and governance state | C2 schema/data plan; later migrations |
| SHA-256 filesystem blob store abstraction | Immutable project content, snapshots, and artifacts | Keep abstraction and development backend | `BlobStore` with filesystem now and S3-compatible backend later | Storage checkpoint |
| Workspace event and immutable snapshot model | Ordered file operations, canonical manifests, replay, and compile inputs | Migrate/evolve | Stable `file_id`, manifest revisions, paper versions, structural operation inverses | Collaboration/version checkpoints |
| TeX compiler (`latexmk`/TeX Live) | Produce PDF/log in isolated worker path | Keep | Versioned exact-state compile behind `Sandbox` | Compile checkpoint |
| Durable PostgreSQL compile queue | Bounded, leased, idempotent worker jobs | Keep and extend scheduling semantics | Coalesced auto-build plus manual jobs, exact manifests, stale classification | Compile checkpoint |
| Templates | Catalog and project instantiation | Migrate | Immutable `template_versions`, pinning, controlled update workflow | Template checkpoint |
| File policies | Path-based editable/read-only/managed restrictions | Replace/evolve | Stable-file, versioned five-state V2 policy model with all-path enforcement | Policy checkpoint |
| Team-project audit and general audit events | Record canonical changes and administrative actions | Keep and normalize | Append-only V2 `audit_events` linked to papers, versions, reviews, and restores | Audit checkpoint |
| Personal projects | User-owned workspaces | Migrate | Writer-only `personal_papers` | C2 planning and migration checkpoint |
| Research Groups | Equal-member shared workspace | Retire after explicit disposition | Writer personal paper, Paper Team, or archive/export per reviewed decision | Data migration checkpoint |
| Teams | Group container that may hold projects | Replace after migration | Admin-created Paper Team with exactly one workspace | Data migration checkpoint |
| Team projects | Canonical project within a Team | Migrate | The single paper workspace intrinsic to a Paper Team | Data migration checkpoint |
| Team membership | Broad group membership and group management | Replace | `paper_team_members` assignment scoped to one paper | Role/team migration checkpoint |
| Additive project role booleans | Writer, Mentor, and Project Manager capabilities per project | Retire after conflicts are resolved | Exclusive global role plus Writer/Mentor Paper Team assignment | Role migration checkpoint |
| Private member drafts and structural changes | Per-member unpublished working tree | Migrate/dispose explicitly, then retire | Shared Yjs/Yrs canonical collaboration plus Writer-scoped undo | Draft-resolution then collaboration checkpoint |
| Publish endpoint/workflow | Merge private drafts into canonical team head | Retire only after every private change is resolved | Durable shared collaboration; no routine Publish step | Draft-resolution then collaboration checkpoint |
| Current textarea editor | Manual edit/save client | Replace | CodeMirror 6 Writer editor; live read-only source projection for Mentor | Writer/Mentor shell checkpoints |
| Current PDF iframe | Display compiled PDF | Replace | PDF.js viewer and annotation overlay with SyncTeX mapping | Review/PDF checkpoint |
| Admin APIs | Global control-plane inspection and user operations | Keep boundary and evolve resources | `/api/admin` V2 users, Paper Teams, templates, policies, restores, review, builds, audit, health | Admin checkpoint |
| CLI/operator boundary | Lifecycle, doctor, backup, user/template administration | Keep and narrow privileged operations | Operator CLI now; future Operator Service for controlled host operations | Operator checkpoint |

Current components remain present until the named future checkpoint migrates data, changes product code, verifies acceptance, and deliberately removes obsolete paths.
