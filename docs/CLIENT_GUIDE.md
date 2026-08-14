# Client guide

Sign in with the email and password supplied by your administrator. Use **New** in the sidebar to create a blank project, import a project ZIP, or make a personal copy of an installed server template.

For a blank project, open `main.tex`, write LaTeX, and save with **Save** or Ctrl/Cmd+S. Select another `.tex` file and use **Set as Main** when needed. Ctrl/Cmd+Enter submits a manual compile. The compact status progresses through Queued and Compiling to Compiled or Failed. Logs and backend-provided errors appear in the bottom panel; a successful PDF opens in the preview pane and can be downloaded.

To import a project, choose **Import Project**, enter a project name, and select a `.zip`. Nested files and binary assets are retained. Binary assets appear in the file hierarchy but are not opened as text. If there is `main.tex` at the archive root it becomes the main file; otherwise exactly one root-level `.tex` is selected automatically. Choose a main file yourself when the project has several plausible documents.

Templates appear only when an administrator has installed them. Creating from one copies its files into your own independent project.
