# AGENTS.md

Entry point for this repository. Keep this file small: rules that every task
needs. Everything else lives in `.agents/*.md`.

Communication with the user: Russian. Code, commits, docs: English.

## Start Here

Read before changing anything:

- [.agents/project.md](.agents/project.md) - what this is, platforms, toolchain.
- [.agents/commands.md](.agents/commands.md) - run, test, build commands.
- [.agents/rules.md](.agents/rules.md) - coding and workflow conventions.
- [.agents/style.md](.agents/style.md) - writing style for all produced text.
- [.agents/todo.md](.agents/todo.md) - current task checklist.

Read when the task touches the area:

- [.agents/architecture.md](.agents/architecture.md) - modules, data flow, gaps.
- [.agents/gotchas.md](.agents/gotchas.md) - traps already hit.
- [.agents/bugs.md](.agents/bugs.md) - known bugs and fixes.
- [.agents/project-decisions.md](.agents/project-decisions.md) - why it is built this way.
- [.agents/models.md](.agents/models.md) - model routing (optional).
- [.agents/skills.md](.agents/skills.md) - repo-scoped skills.
- [.agents/agent-config.md](.agents/agent-config.md) - where agent surfaces live.

## Highest Priority Rules

- Verify with the commands in `.agents/commands.md` before claiming work is done.
- Keep changes minimal; follow existing patterns; no comments unless requested.
- Never edit `proxyinject.dll` / `inject.exe` by hand - rebuild from `src/`.
- Do not commit, push, or create PRs without an explicit user request.
- Never print or commit proxy credentials from `proxy/*` or `proxies.txt`.

## Maintenance Duties

- Something does not build/run/pass -> log it in `.agents/gotchas.md` immediately.
- Session end -> append bugs to `.agents/bugs.md`, decisions to `.agents/project-decisions.md`.
- Every task -> keep `.agents/todo.md` honest.
- Project facts change -> update `project.md`, `commands.md`, `architecture.md`.
