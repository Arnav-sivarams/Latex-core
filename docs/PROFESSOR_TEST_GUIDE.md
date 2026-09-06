# Professor test guide

This walkthrough assumes `./install.sh` completed and printed a healthy URL.

1. Open the URL and sign in as the Admin created by the installer.
2. Build the two importable ZIPs:

   ```sh
   ./examples/professor-demo/make-demo-assets.sh
   ```

3. Open **Admin → Templates** and import `examples/professor-demo/dist/main-template.zip` as the CSE Main Content Template, selecting `main.tex` as its main file.
4. In **Admin → Templates → Front Matter**, import `examples/professor-demo/dist/front-matter.zip`.
5. In **Automatic Defaults**, set both the CSE Main Content Template and CSE Front Matter Pack.
6. If testing SMTP, first replace `student1@example.edu`, `student2@example.edu`, and `mentor@example.edu` in the demo CSVs with real test inboxes.
7. Open **Admin → Imports**. Drag all seven files from `examples/professor-demo/institution/` simultaneously. Select **Add**, then **Review changes**, then **Add records**.
8. Verify that two Student Writer accounts and the assigned Faculty Mentor account were created, a credentials CSV downloaded, and one Team was materialized with Student One as its only Leader, both Writers ordered correctly, and the Faculty Mentor assigned. Confirm the CSE template and Front Matter defaults were applied.
9. If SMTP is configured, confirm temporary-password messages reach the three test inboxes.
10. Sign in as a Writer using the temporary password. Complete **Set your password** with a permanent 12–256 character password.
11. Open Writer sessions for both Students in separate browsers or incognito profiles. Edit the Team paper and verify collaboration converges.
12. Request a manual PDF compile and open the produced PDF.
    Also exercise **Insert → Algorithm Builder**, **Insert → Algorithmic Builder**, and **Insert → Long Table Builder** in a synthetic single-column report. Add the packages the UI names to the preamble and confirm the generated algorithm, multipage table with repeated heading, and final row appear. In a two-column fixture, confirm the Long Table builder explains the incompatibility instead of silently switching layout.
13. As the Team Leader, open **Document details**. Edit the acknowledgement, abstract, and submission date.
14. Compile again and verify Cover, Certificate, Declaration, Acknowledgements, and Abstract pages appear before the main paper.
15. Select **Send for Review**.
16. Sign in as the Mentor. Open the submitted Team report, add feedback in two different `.tex` files, reload, and confirm the toolbar says **Draft saved — not yet visible to writers**.
17. In the Writer context, confirm the drafts are absent. Return to the Mentor and select **Push review**. Confirm the count and submit; wait for **Review submitted. Writers can now see your feedback.**
18. Return as a Writer without refreshing. Open **Reviews**, choose the second item, and confirm the correct file and range open in the editor pane. Select **Done** and confirm the active highlight disappears while Resolved history remains.
19. As Team Leader, send a new review round and verify prior published feedback remains visible. With multiple Mentors, verify one submission does not publish the other's drafts.
20. Create checkpoints, make a later change, and exercise the governed revert/restoration workflow.
21. As Admin, inspect **Build Queue**, **Audit**, and **System**, including backup and restore-drill status.
22. Create two isolated reports and use each report's file-sidebar **Upload image** (`+`) action to upload visibly different PNG files both named `diagram.png`. Confirm each appears at `assets/diagram.png`, each PDF uses its own image, replacing/deleting one does not change the other, and an `images/` legacy fixture still compiles. Repeat while switching reports during a delayed upload and confirm no file or insertion appears in the wrong report.

The demo data is synthetic. Do not import real institutional records into a disposable evaluation system.
