# Manager Changelog

Record each local commit or integrated worker handoff here.

## Entries

- `d3e24fe` - Plan Work ADE platform completion.
  - Added detailed end-to-end execution plan, completion records, and public
    Work/ADE extension decision docs.
  - Reviewed by Ohm and Locke before commit.
- `5dc809d` - Add composable workbench templates.
  - Added Classic/Kanban/Multipane/Review template infrastructure, persisted
    template state, Workbench split panes, Work task board/detail projection,
    and focused frontend tests.
  - Focused Vitest template/projection suite passed after commit.
- `729d953` - Harden Work artifact paths.
  - Added safe relative path validation for Work artifact/file endpoints in
    schema and store tests.
  - Focused `ctx-store agent_work` tests passed after commit.
- `399b29e` - Harden plugin inventory runtime.
  - Marked duplicate plugin IDs as load errors, preserved command stdin
    BrokenPipe handling, and hardened a relative command test.
  - Focused duplicate-plugin daemon test passed after commit.
- `6a36194` - Add safe local validation wrappers.
  - Added `cargo-safe`, `check-local`, and Makefile `check`/safe `test` target.
  - Shell syntax check passed after commit.
- `3d1b60a` - Document Work and plugin contracts.
  - Added manager-owned blocking contracts for Work namespace compatibility,
    Work source-of-truth/storage semantics, ACP provider plugins, and plugin
    contributions/capabilities.
  - This is the base contract commit for future parallel worker branches unless
    superseded by another manager-owned contract commit.
- `8123c74` - Tighten Work plugin contracts.
  - Adds durable diagnostic events, old control-plane historical import
    boundaries, local ACP v1 conformance target, approved importer write actions,
    and ID-class collision rules.
- `b83d8c9` - Add Workbench template visual coverage.
  - Adds Playwright visual coverage for Classic, Kanban, Multipane, Review,
    dense task lists, and multipane split/focus/resize states.
  - Fixes the HTML topbar wrapper so the topbar host owns the shell grid area
    and the template switcher does not collapse into the sidebar column.
