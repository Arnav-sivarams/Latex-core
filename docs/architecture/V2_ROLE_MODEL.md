# V2 Role Model

Status: frozen architecture contract for C2-C21.

## Invariant

`users.role` has exactly three values: `writer`, `mentor`, and `admin`. The role is global and exclusive. A user has exactly one role; roles cannot be composed. Team assignment grants access to a paper but does not add or change a role.

## Writer

A Writer can:

- authenticate into `/write`;
- create and own personal papers;
- access team papers to which an Admin assigned them;
- create, edit, rename, move, and delete team files where file policy permits;
- Set Main where policy permits;
- collaborate live with other Writers and receive Mentor feedback live;
- reply to Mentor review threads and mark Mentor requests as addressed;
- accept or reject Mentor text suggestions;
- compile personal and assigned team papers and view PDFs and artifacts;
- view team history, compare versions, and create named checkpoints;
- immediately undo or redo their own collaborative edits;
- undo their own recent reversible structural operations; and
- request historical restoration of a team paper.

A Writer cannot create a Paper Team, add or remove team members, assign roles, change templates or file protection, directly restore a historical team version, approve their own restoration request, resolve a Mentor thread on the Mentor's behalf, or access `/review` or `/admin`.

### Personal-paper exception

A Writer owns their personal paper. For personal papers only, the owner may restore an older version as a new head. Prior history remains intact.

## Mentor

A Mentor can:

- authenticate into `/review` and access only Admin-assigned Paper Teams;
- view current team source live and view the current PDF;
- manually request compilation of an assigned paper;
- highlight source ranges and PDF regions;
- create comments, questions, change requests, and suggested replacement text;
- reply to review threads;
- assign severity and category, assign feedback to a Writer, and set due dates where supported;
- compare paper versions, inspect activity and authorship, and see changes since the last review;
- mark sections approved, approve a review round, and approve paper review state;
- resolve and reopen feedback;
- review a Writer restoration request, reject it, or endorse it to an Admin; and
- export a review summary.

A Mentor cannot create personal papers or Paper Teams, create files, modify or save source, rename/move/delete files, Set Main, accept their own suggestion, directly restore any paper version, change membership, templates, or file policies, or access `/write` or `/admin`.

Mentor source mutation is rejected by the server. It is not merely hidden in the client.

## Admin

An Admin can:

- authenticate into `/admin`;
- create Writer, Mentor, and Admin users;
- enable or disable users, reset passwords, and safely delete unreferenced users;
- change a user's global role;
- create, archive, or freeze Paper Teams;
- assign and remove Writers and Mentors;
- assign pinned template versions and define or change file policies;
- inspect all paper metadata and history through admin APIs;
- inspect activity, review status, build queue, audit data, system health, and restoration requests;
- apply a Mentor-endorsed team restoration or reject it; and
- execute template updates through the controlled workflow.

An Admin cannot own personal papers, enter `/write`, enter `/review` as a client, edit source, submit CRDT source updates, create Mentor comments, impersonate a Writer or Mentor through the normal UI, or silently bypass team collaboration rules.

Administrative inspection is not team membership.

## Removed V2 concepts

Student and Professor are legacy account types, not V2 roles. Project Manager, Writer-plus-Mentor combinations, Research Groups, user-created or mentor-created Teams, multi-project Teams, private team drafts with Publish, and an Admin writing workspace are not part of V2.
