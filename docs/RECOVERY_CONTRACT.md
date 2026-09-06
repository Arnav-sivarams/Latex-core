# Recovery contract

## A. Process restart with persistent storage intact

A server `DURABLE_ACK` means the content blob has been synchronously published
and its CRDT update batch plus current file reference committed in PostgreSQL.
With the PostgreSQL and BlobStore volumes intact, API, worker, or database
process restart must recover acknowledged source, CRDT snapshots/update logs,
published and draft reviews, accounts, governance history, metadata, templates,
Front Matter, branding, preferences, and durable compilation jobs.

Presence, process IDs, and expired sessions are ephemeral. Docker restart
policy helps availability but is not the durability mechanism.

## B. Client network outage

Before connection, the UI says **Opening…**, not Saved. While a write is
unacknowledged it says Saving, Offline, or Reconnecting. Yjs updates are stored
in origin-scoped IndexedDB under account ID, workspace ID, stable file ID, and
document epoch. Reconnect rechecks server authorization and policy, merges only
the current epoch, and waits for `DURABLE_ACK` before showing Saved.

If access becomes read-only or a restore advances the document epoch, cached
text is not submitted or merged. The UI offers recovery text for copying. If
IndexedDB/local storage is unavailable or full, the UI states that local
recovery is unavailable. Browser persistence does not survive clearing site
data, losing the profile, or losing the device.

## C. Lost server storage

Recover into a new isolated installation from the latest backup directory that
passes `verify-backup.sh`. The available recovery point is the timestamp in its
manifest. The operation restores PostgreSQL and the BlobStore together and
verifies every referenced blob before serving. There is no promise of zero
loss for acknowledgements after the last completed off-host backup.
