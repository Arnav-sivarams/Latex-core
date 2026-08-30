# V2 Migration Policy

Status: freeze of future migration decisions and unresolved cases. No migration is performed by C1.

## C0 inventory constraints

C0 recorded 147 users, 73 personal projects, 43 Teams, 41 team projects, 115 team memberships, 97 project-role memberships, one Research Group, five templates, and seven file-policy rules. It also found 25 users with role conflicts, seven multi-project Teams, 13 users with unpublished private work, and 13 unpublished change sets.

Future tooling must inventory, classify, dry-run, and reconcile these records without identities in reports intended for architecture review. Counts must be rechecked against the frozen source and every resolution must be explicit and auditable.

## Legacy account types

The legacy `student`, `professor`, and `admin` account types must not be blindly mapped.

- A Student is likely a Writer, but conversion is not automatic when role history, ownership, or project assignments contradict that result.
- A Professor is not automatically a Mentor. Ownership and actual use require explicit resolution.
- An Admin remains an Admin only after explicit validation.
- The four users without credential/account-type rows are reported as a separate exception class and are not inferred from other records.

Every user must end with exactly one validated V2 role or a recorded archive/exclusion decision before cutover.

## Overlapping project roles

C0 found 25 users across 45 project-membership records with overlapping roles. A legacy Writer-plus-Mentor assignment must not be collapsed automatically. Each conflict requires an explicit global-role decision and compatible ownership/assignment changes. Project Manager grants have no V2 counterpart and require explicit administrative reassignment rather than capability carry-over.

## Multi-project Teams

C0 found seven Teams containing multiple projects. V2 requires one Paper Team per paper. Future migration splits each legacy team project into its own Paper Team while preserving applicable membership, source state, history, template provenance, policy, compile records, and audit linkage. The dry run must show the proposed split and exceptions before writes.

## Research Group

C0 found one legacy Research Group. A future explicit decision must either convert its workspace to a Writer-owned personal paper, convert it to a Paper Team, or archive/export it. It is never silently deleted and Research Groups do not remain a V2 product concept.

## Unpublished private work

C0 found 13 users with unpublished private work and 13 unpublished change sets, comprising 16 draft files and 17 structural operations. Before retiring the private Publish workflow, every change set must be merged, exported, archived, or explicitly rejected with a recorded Admin decision. No draft file or structural operation is discarded implicitly.

Migration tooling must preserve enough base-revision and actor context to review conflicts and prove the disposition of each change.

## Personal papers

Only a Writer may own a V2 personal paper. Personal papers owned by a legacy Professor, Admin, or user selected as Mentor must be transferred to a Writer, converted to a Paper Team, or archived before role conversion. No ownership is fabricated from recent access.

## Safety and acceptance

Migration is staged as read-only inventory, deterministic proposal, reviewed resolution input, dry-run validation, backup verification, and only then a separately authorized apply checkpoint. It must be idempotent or resumable, preserve append-only history, produce structured errors, and never delete source or private work as a side effect of role simplification.
