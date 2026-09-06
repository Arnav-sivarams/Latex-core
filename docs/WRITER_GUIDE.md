# Writer guide

## Document details

For a Team report, use the compact **Document details** toolbar action to view its Front Matter Pack, status, section choices, automatic metadata, and custom fields. The Team Leader can toggle optional sections and save fields permitted by the pack. Required sections stay enabled. Regular Team Writers have the same view without editing controls; institution-assigned Front Matter cannot be removed by a Writer.

Saving document details flushes current collaboration, safely escapes all entered text, atomically rebuilds hidden Front Matter files, adds one workspace revision/history boundary, and requests a normal automatic build. These generated files never appear in the file tree or enter an editable Yjs room. See [Front Matter Packs](FRONT_MATTER_PACKS.md).

Writers use `/write` for personal papers and assigned Team reports.

## Papers and files

- Create and own personal papers from the Writer paper list.
- Open only Team reports to which an Admin assigned you.
- Create, rename, delete, and select the main file when the Team lifecycle and current file policy permit it.
- File identity is stable across renames and version history.

The five policies are `EDITABLE`, `CONTENT_READ_ONLY`, `STRUCTURE_LOCKED`, `TEMPLATE_MANAGED`, and `HIDDEN_SYSTEM`. The server enforces them on HTTP and collaboration connections. Hidden system files are omitted from the normal tree.

## Realtime, offline, and saving

Source changes synchronize through durable collaboration rooms. **Save** and `Ctrl/Cmd+S` request durable CRDT synchronization; they do not compile or create a checkpoint. The indicator distinguishes Saving, Saved, Offline, and Reconnecting states.

Offline state is stored per paper epoch. After a governed restoration, changes from the previous epoch are preserved locally and are not merged into the restored paper. Use **Copy recovery text** to retrieve that buffer.

## Build and PDF

After a durable local or remote source change becomes idle for about two seconds, the client requests an automatic exact-state build. The server remains authoritative for state barriers, hash deduplication, and queue coalescing. `Ctrl/Cmd+Enter` runs a manual compile. The last good PDF remains available when a later build fails.

## History and restoration

History contains immutable versions. A Writer may directly restore an owned personal paper after confirmation; the old head is first retained as a safety version and restoration creates a new head.

For a Team report, a regular Writer chooses **Request Revert**, selects a version, optionally enters a reason, and submits it to the Team Leader. The Leader can reject or safely apply the request. A Leader may also directly choose **Revert** after explicit confirmation. Each applied revert first records `PRE_RESTORE_SAFETY`, advances the document epoch, and creates a new head without deleting later history.

## Reviews and productivity

The Team Leader can use **Send for review** only when the exact current source has a successful matching PDF. **Withdraw review** cancels an unfinished round without publishing any Mentor drafts. Writers receive feedback only after a Mentor uses **Push review**. The **Reviews** toolbar count includes published unresolved feedback even after its round closes. Selecting an item opens its stable file identity at the anchored range; if source drift prevents a trustworthy mapping, the original excerpt and reviewed PDF location are shown instead of highlighting unrelated text. **Done** resolves and hides one highlight without deleting history, and **Apply** uses the Writer-attributed suggestion flow.

The file sidebar calls new uploaded-image storage **Assets**. Existing paths such as `images/chart.png` remain unchanged and continue to be shown truthfully. The sidebar no longer contains an Outline tab.

**Editor settings** changes only the signed-in account's source-editor font size (12–26 px) and Light/Dark editor theme. Reconfiguration keeps the active collaboration document, undo history, review decorations, and the surrounding light application shell intact.

The workspace keeps Papers/Files, source, and PDF as three independently scrolling panes. Save, Compile, Insert, Math, Problems, History, and Reviews are available from the compact toolbar. Problems, immutable versions/revert controls, and active/resolved comments open in temporary drawers. File Rename, Delete, and Set Main live in the selected file's ellipsis or right-click menu.

Math is a searchable first-class palette (`∑`) covering common symbols, descriptions, components, and equation/matrix templates. Selection inserts through the same Writer-local Yjs transaction as the existing safe builders. `Ctrl/Cmd+P` opens files, `Ctrl/Cmd+K` opens commands, and a Team Leader can use `Ctrl/Cmd+Shift+R` to send an eligible paper for review.
