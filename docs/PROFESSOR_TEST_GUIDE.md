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
13. As the Team Leader, open **Document details**. Edit the acknowledgement, abstract, and submission date.
14. Compile again and verify Cover, Certificate, Declaration, Acknowledgements, and Abstract pages appear before the main paper.
15. Select **Send for Review**.
16. Sign in as the Mentor. Open the submitted Team paper, add a comment, and add a suggestion.
17. Return as a Writer. Confirm the review highlight, respond as needed, and mark the thread **Done**.
18. As Team Leader, exercise **End Review**, then reopen/send another round and verify previous review history remains visible.
19. Create checkpoints, make a later change, and exercise the governed revert/restoration workflow.
20. As Admin, inspect **Build Queue**, **Audit**, and **System** for the actions performed and healthy service state.

The demo data is synthetic. Do not import real institutional records into a disposable evaluation system.
