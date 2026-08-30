# V2 product cutover

The V1 data schema and all legacy records remain intact for migration work, but the V1 browser product is retired. `/workspace` and every legacy product API deny authenticated access; no legacy editor, Team, Research Group, draft, publish, or source-mutation workflow is user-accessible.

Final browser routes are exclusive: Writer uses `/write`, Mentor uses `/review`, and Admin uses `/admin`. An authenticated non-Admin account without a `global_user_roles` assignment is sent to `/account-setup`, which provides only an explanation and logout. A legacy Admin may use `/admin` temporarily for cutover operations.

Roles are never inferred from legacy account types. Admin lists assigned and unassigned canonical users in V2 USERS and explicitly assigns exactly one Writer, Mentor, or Admin role using the invariant-checked operation that revokes existing sessions. Only a V2 Admin can create Paper Teams.

Legacy Teams and Research Groups remain migration data, not V2 product features. They may be inspected from clearly labeled Admin operational views, but Writer and Mentor shells contain no creation or management controls.
