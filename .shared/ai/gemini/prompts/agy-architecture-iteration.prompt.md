# AGY Architectural Iteration

Execute the prompt @[.github/prompts/architecture-iteration.prompt.md] with the following inputs from the Project Initiator:

- The document(s) are in @docs/vision/
- Lead Developer model: `gemini-3.8-flash-high` if not specified otherwise
- Architect model: `claude-opus-4-6-thinking` if not specified otherwise
- Create a uniquely named `prompt-file.txt` in your session scratch pad to pass to the agent.
- Use `agy --model [model] --print "$(cat prompt-file.txt)"` to execute the prompt with the specified models.
- The models stated are pre-verified as available in the current environment. You do not need to test them before execution.
- **DO NOT** search your chat history or invocation context; all necessary inputs are provided in this prompt.
