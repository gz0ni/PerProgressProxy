# Model Catalog and Routing

This project does not route LLM calls itself; the table is guidance for agents
working on the repository.

## Routing

| Task class | Model to propose |
|---|---|
| Default coding | session default |
| Complex refactoring, planning | strongest available |
| Long autonomous work | strongest available |
| Vision / UI debugging | any multimodal model |
| Code review | strong reasoning model |
| Exploration / research | fast, cheap model |
| Routine (tests, simple edits) | fast, cheap model |

## Workflow

1. Classify the task against the table.
2. If the session model does not fit, propose a switch in one line.
3. Delegate exploration and review to subagents.
4. Do not burn top-tier models on routine work.
