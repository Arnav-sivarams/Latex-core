# LaTeX Core V2 Architecture

Status: C1 master architecture freeze. Detailed contracts linked below govern C2-C21; C1 contains documentation only.

## Product model

V2 has three global, exclusive roles:

- **Writer** works in `/write`, owns personal papers, and collaborates in assigned team papers.
- **Mentor** works in `/review`, reads assigned team papers live, and authors review feedback without source mutation.
- **Admin** works in `/admin`, manages the control plane without entering client writing or review roles.

One Paper Team equals one paper and one workspace. Admin creates Paper Teams and assigns Writers and Mentors. Membership grants access; global role determines capability. See [Role Model](V2_ROLE_MODEL.md), [Domain Model](V2_DOMAIN_MODEL.md), and [Client Shells](V2_CLIENT_SHELLS.md).

## System diagram

```text
 Writer browser (/write) -----------+
   CodeMirror 6 + Yjs               |
                                     v
 Mentor browser (/review) ---> Collaboration Gateway ---> PostgreSQL
   read-only source + review        Yrs room actors        state, queue,
   PDF.js annotation overlay        authenticated WS       updates, snapshots
              |                             |               versions, review
              |                             +----> BlobStore (SHA-256)
              |                                      |
              +---- SyncTeX mapping <--- artifacts <-+-- Compile scheduler
                                                         |
                                                         v
                                                   TeX worker/Sandbox
                                                   TeX Live + latexmk

 Admin browser (/admin) ---> Control API (/api/admin) ---> PostgreSQL/BlobStore
                                      |
                                      +-- future Operator Service
                                          (controlled host operations)
```

The API, parser, and collaboration gateway never execute user LaTeX. PostgreSQL is authoritative for structured state and the durable compile queue. Blob content and immutable artifacts are SHA-256 addressed through `BlobStore`.

## Primary data flow

```text
Writer edit -> authenticated collaboration -> durable update -> broadcast
            -> coalesced exact-state auto-build -> PDF/SyncTeX
            -> Mentor live review and anchored feedback
```

The update path acknowledges “Synced” only after durable persistence. Compilation is separate, begins after an initial idle debounce of approximately two seconds, and never promotes stale output. See [real-time collaboration](adr/ADR-001-REALTIME-COLLABORATION.md), [performance](adr/ADR-002-COLLABORATION-PERFORMANCE.md), [WebSocket protocol](V2_WEBSOCKET_PROTOCOL.md), and [versioned compile](adr/ADR-005-VERSIONED-COMPILE.md).

## Undo and restoration

Writer-scoped collaborative undo reverses the Writer's own recent edits and is immediately visible. Reversible structural undo is likewise actor-scoped and policy-checked. It is distinct from historical whole-paper restore.

Team restoration follows:

```text
Writer request -> Mentor comparison and rejection/endorsement -> Admin rejection/apply
```

Apply creates a new head in a new document epoch and retains every prior version. See [Undo Versus Restore](adr/ADR-003-UNDO-VS-RESTORE.md) and [Team Restoration](adr/ADR-004-TEAM-RESTORATION.md).

## Review, templates, history, and security

Mentor review uses CRDT-relative source anchors and persistent PDF artifact geometry, with SyncTeX mappings that explicitly represent approximation and drift. Suggested replacements mutate source only after Writer acceptance and carry Writer attribution. Paper Teams pin immutable template versions; controlled updates checkpoint and enforce five explicit file-policy states. History has separate CRDT logs, snapshots, human-visible versions, and audit layers.

Detailed decisions are [Source/PDF Anchors](adr/ADR-006-SOURCE-PDF-ANCHORS.md), [Mentor Review](adr/ADR-007-MENTOR-REVIEW.md), [Templates and Policies](adr/ADR-008-TEMPLATES-AND-POLICIES.md), [Version History](adr/ADR-009-VERSION-HISTORY.md), [Review State Machine](V2_REVIEW_STATE_MACHINE.md), and [Security Boundaries](adr/ADR-010-SECURITY-BOUNDARIES.md).

## Migration summary

C0 recorded 147 users, 73 personal projects, 43 Teams, 41 team projects, 115 team memberships, 97 project-role memberships, one Research Group, five templates, and seven file-policy rules. Risks include 25 users with conflicting role history, 45 affected project-membership records, seven multi-project Teams, four users lacking credential/account-type rows, and unpublished private work for 13 users across 13 change sets, 16 files, and 17 structural operations.

No category is blindly converted or discarded. Role conflicts require explicit resolution; multi-project Teams split per paper; the Research Group receives an explicit conversion/archive choice; and every unpublished change receives a recorded disposition. See [Migration Policy](V2_MIGRATION_POLICY.md) and [Component Migration Map](V2_COMPONENT_MIGRATION_MAP.md).

## Checkpoint dependencies

C2 begins legacy-data migration planning and dry-run tooling; it must consume the role, domain, migration, and component-map contracts without implementing unrelated product features. Subsequent C3-C21 checkpoints must sequence schema/data transition, server authorization, collaboration persistence and protocol, Writer/Mentor/Admin shells, review and anchor layers, version/restore governance, compile coalescing, templates/policies, operational hardening, and acceptance testing. A later checkpoint may refine engineering parameters only through measured evidence and an explicit superseding architecture decision.

Targets for those checks are in [Nonfunctional Targets](V2_NONFUNCTIONAL_TARGETS.md). C1 itself introduces no Rust, JavaScript, HTML/CSS, SQL, configuration, migration, Compose, or CLI change.
