# C0 Prototype Baseline

- Frozen commit: `2fe37787e86d7bc3ac10bd761549466ea6a8e0d2` (`2fe3778`)
- Prototype tag: `prototype-v1-before-role-simplification`
- V2 branch: `simplified-paper-platform-v2`
- Runtime status at freeze: Running; database, queue, compiler, and one worker running; doctor healthy

## Architecture Being Retired

- Student/Professor/Admin account roles
- Research Groups
- Additive Writer/Mentor/Project Manager team roles
- Private team drafts plus Publish
- Multiple projects per Team

## Preservation

- Database backup created and verified
- Schema backup created and verified
- Blob backup created and verified
- Unpublished drafts inventoried
- Role conflicts inventoried
- Multi-project Teams inventoried
- External backup path: `/home/arnav/Projects/Backups/latex-core/c0-prototype-20260830-071128`

## Inventory Counts

- Users: 147 (95 student, 28 professor, 20 admin, 4 without a credential/account-type row)
- Personal projects: 73
- Teams: 43
- Team projects: 41
- Team memberships: 115
- Project role memberships: 97
- Research Groups: 1
- Templates: 5
- File-policy rules: 7
- Users with role conflicts: 25 (45 project-membership records)
- Multi-project Teams: 7
- Users with unpublished private work: 13
- Unpublished project-user change sets: 13 (16 draft files and 17 structural operations)

No prototype data was migrated or deleted during C0.
