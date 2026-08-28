# AGENT EXECUTION PARAMETERS & RESTRAINTS

## 1. FILE READING & INSPECTION
- NEVER use bash terminal commands like `cat`, `head`, `tail`, or `less` to view files.
- NEVER pipe file contents into other tools (e.g., `cat file.py | grep`).
- ALWAYS use the native file-viewing or code-search tools provided in your toolset.

## 2. FILE MODIFICATION & PATCHING
- NEVER use inline Python scripts (e.g., `open(f, "w").write(...)`) to rewrite code files.
- NEVER use bash stream editors (`sed`, `awk`, `echo >>`) to inject code.
- ALWAYS use specific line-by-line block replacement or patch tools.
- DO NOT rewrite an entire file if you are only changing a few lines.
- CRITICAL: Read and parse the target file completely before staging any edits to prevent truncation.

## 3. CODE SEARCH & GREP USAGE
- NEVER execute raw terminal `grep` commands.
- ALWAYS use the built-in structured codebase search or indexing tools.
- Keep search queries specific to exact symbol names, class names, or function definitions to prevent context token flooding.

## 4. ERROR HANDLING & SAFETY LOCKS
- If a file write or patch fails, STOP immediately. Do not attempt a terminal-based fallback.
- If you run into an infinite loop of failing commands, pause and ask the human user for clarification.
- Verify syntactical correctness of any modified file before marking a task as complete.
