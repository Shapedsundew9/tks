# Common Instructions

## Documentation

- Keep `README.md` aligned with the install, usage, development, and publishing workflows.
- Always run `npx markdownlint-cli2 --fix "**/*.md"` after markdown changes but do not address issues that cannot be fixed without explicit permission.
- Never modify `.markdownlint-cli2.jsonc`. Ask the user first if a change is necessary.

## CLI Output Rules

- Whenever you generate Mermaid diagrams, architecture charts, or markdown explanations exceeding 15 lines, DO NOT dump the raw markdown solely into the terminal.
- Always write or update the output directly into `scratchpad/` creating the folder as necessary.
- In the terminal, provide only a 1–2 sentence summary and state that the full diagram/plan has been written to the scratchpad.
