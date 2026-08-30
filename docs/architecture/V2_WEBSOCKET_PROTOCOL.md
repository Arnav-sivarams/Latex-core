# V2 WebSocket Protocol Contract

Status: semantic contract only. Exact framing, serialization, numeric discriminants, and compression negotiation remain implementation decisions.

## Connection and envelope

The transport is an authenticated binary WebSocket. Authentication is validated at connect and remains subject to role, membership, status, and policy revocation. Typed metadata envelopes identify the message category, protocol version, `paper_id`, optional `file_id`, authenticated session identity, `document_epoch`, and a client or server message sequence where ordering or durable acknowledgement requires it. CRDT updates are binary payloads rather than JSON-encoded update bytes.

The authenticated server session is authoritative; a client-supplied user identity is never trusted. A text-file room is `paper:{paper_id}:file:{file_id}`.

## Client to server

- `AUTH/JOIN`: authenticate and request access to a paper/file room.
- `SOURCE_UPDATE`: a Writer's binary CRDT update plus transaction metadata.
- `AWARENESS_UPDATE`: ephemeral cursor, selection, or presence state.
- `STRUCTURAL_COMMAND`: a Writer request to create, rename, move, delete, or Set Main.
- `SYNC_REQUEST`: request initial or incremental state for the current epoch.
- `COMPILE_REQUEST`: manually request an exact-state compile where authorized.

A Mentor client has no permitted `SOURCE_UPDATE` or `STRUCTURAL_COMMAND`. An Admin never uses the ordinary collaboration protocol to mutate source.

## Server to client

- `JOIN_ACCEPTED`: confirms role, scope, room, epoch, and effective capabilities.
- `INITIAL_STATE`: snapshot plus update tail or equivalent current CRDT state.
- `SOURCE_UPDATE`: accepted remote CRDT update.
- `AWARENESS_UPDATE`: coalescible ephemeral presence.
- `STRUCTURAL_EVENT`: authoritative manifest/file operation result.
- `DURABLE_ACK`: confirms specified Writer update sequence is durably persisted.
- `POLICY_CHANGED`: reports a new effective file-policy capability.
- `MEMBERSHIP_REVOKED`: terminates access to the affected paper.
- `BUILD_STARTED`, `BUILD_FINISHED`, `BUILD_FAILED`: compile lifecycle events.
- `PDF_CURRENT_CHANGED`: identifies the newest promoted valid PDF.
- `REVIEW_EVENT`: authorized review-thread, suggestion, or approval change.
- `PAPER_EPOCH_CHANGED`: invalidates old state after restoration and requires reload.
- `ERROR`: typed rejection with correlation information where appropriate.

## Durability and ordering

Receiving a `SOURCE_UPDATE` does not itself mean durable sync. The gateway validates session, Writer role, paper membership, status, and file policy; applies the update to the in-memory Yrs document; broadcasts it; batches persistence; and emits `DURABLE_ACK` only after the covered update sequence is committed to PostgreSQL. A client displays “Synced” only for durably acknowledged state.

Sequences are scoped so reconnect can determine which updates are durable and which must be retransmitted idempotently. `document_epoch` prevents an update authored against a pre-restoration document from entering the new head.

## Revocation

Role, membership, team status, and policy changes invalidate affected active capability. Membership removal closes the affected paper session. A rejected mutation is never broadcast as accepted and never receives a durable acknowledgement.
