# UI Artifact Reviews

Record screenshot/video artifacts and visual review findings.

## Reviewed Artifact Sets

- Classic template: initial desktop-wide screenshot exposed a topbar grid-area
  wrapper bug; after the host fix, the refreshed screenshot was manually viewed
  and accepted. Artifact:
  `/tmp/ctx-3c22f3412cbc/volatile/tmp/ctx-e2e-visual-data-3758079/argos-screenshots/workbench-template-classic-dark-desktop-wide.png`.
- Kanban template: desktop-wide and narrow screenshots were manually viewed and
  accepted. Lanes remain readable at desktop width and become a horizontal board
  at narrow width without squeezing cards into unreadable columns. Artifacts:
  `/tmp/ctx-3c22f3412cbc/volatile/tmp/ctx-e2e-visual-data-3762676/argos-screenshots/workbench-template-kanban-dark-desktop-wide.png`,
  `/tmp/ctx-3c22f3412cbc/volatile/tmp/ctx-e2e-visual-data-3762676/argos-screenshots/workbench-template-kanban-dark-narrow.png`.
- Multipane template: resize/focus screenshot was manually viewed and accepted.
  Focus ring, split ratio, and empty secondary pane render without text
  collision. Artifact:
  `/tmp/ctx-3c22f3412cbc/volatile/tmp/ctx-e2e-visual-data-3762676/argos-screenshots/workbench-template-multipane-resized-right-dark-desktop-wide.png`.
- Review template: desktop-wide screenshot was manually viewed and accepted.
  Summary metrics, active task pane, and Work detail area preserve hierarchy.
  Artifact:
  `/tmp/ctx-3c22f3412cbc/volatile/tmp/ctx-e2e-visual-data-3762676/argos-screenshots/workbench-template-review-dark-desktop-wide.png`.
- Dense task list: desktop-wide screenshot was manually viewed and accepted for
  task-list density and empty main content. Artifact:
  `/tmp/ctx-3c22f3412cbc/volatile/tmp/ctx-e2e-visual-data-3762676/argos-screenshots/workbench-template-classic-dense-task-list-dark-desktop-wide.png`.
- Plugin-contributed panel/template: desktop-tight and narrow Kanban screenshots
  were manually viewed after the contribution-row layout fix. Source labels,
  captions, and badges remain readable, and the narrow layout uses intentional
  panel scrolling rather than horizontal overflow. Artifacts:
  `/tmp/ctx-3c22f3412cbc/volatile/tmp/ctx-e2e-visual-data-42467/argos-screenshots/workbench-contributions-panel-ready-dark-desktop-tight.png`,
  `/tmp/ctx-3c22f3412cbc/volatile/tmp/ctx-e2e-visual-data-42467/argos-screenshots/workbench-contributions-kanban-narrow-dark.png`.
- Final Workbench visual rerun: 20 Playwright visual tests passed with the
  latest screenshot set under
  `/tmp/ctx-3c22f3412cbc/volatile/tmp/ctx-e2e-visual-data-834633/argos-screenshots`.
- Source-labeled command surfaces: manually sampled
  `/tmp/ctx-3c22f3412cbc/volatile/tmp/ctx-e2e-visual-data-834633/argos-screenshots/workbench-commands-source-labels-dark-desktop-tight.png`.
  Provider and plugin rows carry row-specific source labels without changing
  command routing.
- Unsupported contribution diagnostics: manually sampled
  `/tmp/ctx-3c22f3412cbc/volatile/tmp/ctx-e2e-visual-data-834633/argos-screenshots/workbench-contributions-unsupported-diagnostics-dark-desktop-tight.png`.
  Unsupported renderer/template state is visible as host-owned diagnostic data,
  not executable plugin UI.
- Hot reload add/change/remove states: manually sampled the sequence under
  `/tmp/ctx-3c22f3412cbc/volatile/tmp/ctx-e2e-visual-data-834633/argos-screenshots`:
  `workbench-plugins-hot-reload-empty-dark-desktop-tight.png`,
  `workbench-plugins-hot-reload-added-dark-desktop-tight.png`,
  `workbench-plugins-hot-reload-changed-label-dark-desktop-tight.png`, and
  `workbench-plugins-hot-reload-removed-fallback-dark-desktop-tight.png`.
  The removed-plugin fallback preserves the active composer draft.
- Mobile shell: manually sampled
  `/tmp/ctx-3c22f3412cbc/volatile/tmp/ctx-e2e-visual-data-834633/argos-screenshots/workbench-template-classic-responsive-dark-mobile-narrow.png`.
  The visual now exercises mobile Tauri shell behavior, including task-list
  drawer selection and collapsed final layout, rather than a squeezed desktop
  sidebar.

## Accepted Missing Artifact Sets

- Import/export errors and redaction preview screenshots are deferred because
  those are CLI/store surfaces in the current local branch, not a Workbench UI
  flow yet.
- Plugin provider diagnostics beyond inert Workbench contribution diagnostics
  are deferred until the daemon-connected plugin apply/reload and diagnostics UI
  lifecycle slice.
