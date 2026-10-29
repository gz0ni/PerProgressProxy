# Agent Configuration Model

## Surfaces

- `AGENTS.md` - auto-loaded entry point: always-on rules and links.
- `.agents/*.md` - reference docs linked from `AGENTS.md`.
- `.agents/skills/<name>/SKILL.md` - repo-scoped skills, loaded on demand.
- Global user-level skills live outside the repository and are not duplicated here.

## Placement Rules

- Put stable conventions in `AGENTS.md` only when every task must see them.
- Put explanations and maps in `.agents/*.md`.
- Put reusable workflows in `.agents/skills/<name>/SKILL.md`.
- Prefer tests over prose where a rule can be enforced mechanically.

## Skill Authoring Rules

- Lowercase hyphenated names.
- Descriptions start with `Use when...`.
- Keep `SKILL.md` lean; link to `.agents/*.md` for reference material.
