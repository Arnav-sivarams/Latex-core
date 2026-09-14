# Manual compile, review lock, and institutional API handoff

## Integrated behavior

Compilation is explicit/manual throughout the Writer client and application persistence layer. Durable save/Yjs, files and assets, Front Matter, restore, creation/open/reconnect, review submission, timers, and institutional reads do not submit builds. `POST /api/v2/papers/{paper_id}/builds` accepts only `trigger_type=manual`, captures exact durable state under the workspace mutation lock, and retains queue caps, idempotency, progress, and last-good artifacts.

Already queued/running jobs and build history are retained. A legacy in-memory scheduler row whose pending candidate is marked `auto` is cleared when the active job completes instead of creating a new job; a pending explicit `manual` candidate retains the normal coalesced continuation.

The active `review_rounds.status = OPEN_FOR_REVIEW` row is the sole lock authority. It is consulted in common HTTP mutation helpers, version/restore/template/Front Matter operations, compile admission, collaboration join, and each collaboration write. A Yjs update is applied to a private candidate, authorized and persisted under the shared workspace lock, and only then applied/broadcast to the room. Local capability events close already-open connections; another API process or a reconnect is protected by the PostgreSQL recheck.

| Current state | Writer | Team Leader | Mentor | Exit/result |
| --- | --- | --- | --- | --- |
| Editable/no open round | Existing role/file-policy edits, manual Compile | Same plus checkpoint/revert and Send for review after exact-PDF validation | Assigned read access | Leader sends exact current build |
| `OPEN_FOR_REVIEW` | Read/navigation/download/published feedback only | Same read-only report access plus End review | Private drafts and Push review per existing participation | One Mentor push publishes only that Mentor; final required push closes, or Leader ends |
| Closed | Normal prior permissions restored | Normal prior Leader permissions restored | Historical/published read access | A later exact manual build can begin a new round |

End review marks pending Mentor participations withdrawn but neither publishes nor deletes their draft rows. A last-required-Mentor push closes normally. No database transaction remains open for the review duration.

The external API is mounted at `/api/integration/v1/`; Admin credential lifecycle is under `/api/admin/integration/v1/clients`. It uses 256-bit one-time bearer credentials with SHA-256 verifiers, live expiry/revocation, explicit scopes and report/institution-wide coverage, allowlisted DTO projections, bounded cursor pages, cross-process database rate limiting, and immutable report/version/file/build identities. See [the complete API guide](INSTITUTIONAL_API.md) and [OpenAPI](openapi/institutional-api-v1.yaml).

## Qualification evidence

The focused populated-Team database test `manual_build_review_lock_and_integration_reads_share_exact_state` recorded zero queue growth for ordinary save, exactly one job for the explicit manual request, zero additional jobs for denied review operations, read-only collaboration for both Writers, restored read/write after Leader close, and a stale `is_current=false` PDF after a later source edit. It also checks scoped report/people/version/build projections, contact omission, minimal auditing, and immediate revocation.

Against the live isolated API, the example client downloaded `main.tex` and the existing PDF by explicit immutable identity. Their SHA-256 values were respectively `34b45de624113da241a852ac890bc992f3ee151e0dbccfd8e59f5bb5a1676b9f` and `46842484c05398e51ea6d0afc37dfdb2bba15019284dc87e41610153702195f8`; the PDF response identified version `8b4a7305-5703-44de-b407-52f5f6e19ad6` and `is_current=false`. Pre/post API-read counters were unchanged at 25 compile jobs, 1 outstanding job from other isolated fixtures, aggregate workspace version 32; only minimal access rows increased (17 to 21). The negative matrix returned invalid token 401, malformed cursor/limit 400, missing scopes 403, out-of-scope/cross-file IDs 404, and machine mutation 401. Revocation then returned 401 without API restart.

One selected compiler test generated a `%PDF-` artifact through frozen M7 image `latex-core-texlive@sha256:8db804f76b8e80e5be9fb28ba14b0938df5989b7a8250ca6b0e9f3c200c4ee38`. The full historical M7 corpus was intentionally not run.

## Migration and update

Migration `0027_institutional_read_api.sql` is additive. Follow [Server updates](UPDATE_SERVER.md): verified off-host database/BlobStore backup, build candidate API/Worker images, run the candidate embedded migrator, verify the migration ledger, then replace API and Worker. Preserve the current `.env`, Compose project/volumes, Caddy endpoint, and 9000/9001 host-port configuration.

The v1 listing is live pagination, not a global point-in-time export. It intentionally provides no incremental deletion/change-feed guarantee; archive clients should periodically reconcile all permitted pages with idempotent upserts. Older version manifests without captured structured metadata report `not_recorded`.
