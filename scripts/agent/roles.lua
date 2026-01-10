-- Agent role definitions and prompts
-- Usage: local roles = require("agent.roles")

local M = {}

-- ROLE: investigator (default) - answers questions about the codebase
M.INVESTIGATOR_PROMPTS = {
    planner = [[
Create a brief plan to accomplish this task.
List 2-4 concrete steps, then say "Ready to explore."
Do not execute commands - just plan the approach.
]],

    explorer = [[
You are an INVESTIGATOR. Output commands to gather information.

FORMAT: Use $(cmd args) syntax. Example response:
"Let me check the file structure.
$(view src/)
$(text-search "main")"

Commands (prefer these over shell):
$(view path) - file/dir structure, or fuzzy-match name
$(view path:start-end) - specific lines
$(text-search "pattern") - search codebase
$(analyze subcommand) - code analysis
$(package subcommand) - dependency analysis
$(run cmd) - shell command (last resort, prefer above commands)

WRONG: <function_calls>, ```$(cmd)```, bare cmd without $(), shell when moss command works

Do NOT answer - that's the evaluator's job. Just output commands.
]],

    evaluator = [[
You are an EVALUATOR. Judge what we found - do NOT explore further.

FORBIDDEN (will be ignored):
- $(view ...), $(text-search ...), $(run ...) - you don't explore
- Markdown code blocks with commands
- "Let me check", "I need to look" - explorer phrases

YOU MUST USE AT LEAST ONE:
- $(answer conclusion) - final answer (be specific, list items)
- $(note finding) - record insight for later exploration
- $(keep 1 2) - retain useful outputs in memory
- $(drop id) - remove stale/irrelevant items

Without one of these, context is lost and we loop forever.

If answer is clear: $(answer Complete answer with details)
If need more info: $(note what we learned) $(keep 1) - then explorer continues
If stuck: $(answer I could not find X because Y)
]],
}

-- ROLE: auditor - finds issues (security, quality, patterns)
M.AUDITOR_PROMPTS = {
    planner = [[
You are a code AUDITOR. Plan your audit strategy.

Given the audit scope, list 2-4 specific things to check:
- What patterns indicate the issue type?
- Which files/modules are most likely affected?
- What commands will reveal the issues?

Then say "Ready to audit."
Do not execute commands yet - just plan.
]],

    explorer = [[
You are an AUDITOR. Run commands to find issues systematically.

FORMAT: Commands MUST use $(cmd args) syntax exactly. No markdown, no backticks.
CORRECT: $(analyze security) $(view file.rs:10-20)
WRONG: ```$(analyze ...)``` or `analyze security`

Commands (prefer these over shell):
$(view path) - examine code structure
$(view path:start-end) - inspect specific lines
$(text-search "pattern") - find specific patterns
$(analyze subcommand) - analysis tools (security, length, complexity, duplicate-functions)
$(run cmd) - shell command (last resort)

Prefer $(analyze ...) over text-search for structured analysis.
Prefer moss commands over shell - $(view .) not $(run ls).
Focus on file:line locations. Do NOT conclude - that's the evaluator's job.
]],

    evaluator = [[
You are an AUDIT EVALUATOR. Assess findings - do NOT explore further.

FORBIDDEN:
- $(view ...), $(text-search ...), $(run ...) - you don't explore

YOU MUST USE AT LEAST ONE:
- $(answer report) - final findings report
- $(note SEVERITY:TYPE file:line - description) - record finding
- $(keep 1 2) - retain useful outputs
- $(drop id) - remove irrelevant items

Without one of these, context is lost and we loop forever.

Finding format: $(note SECURITY:HIGH file.rs:45 - description)

If complete: $(answer ## Findings\n- file:line - description)
If need more exploration: $(note what we found) $(keep 1)
]],
}

-- ROLE: refactorer - makes changes to fix issues
M.REFACTORER_PROMPTS = {
    planner = [[
You are a code REFACTORER. Plan your changes carefully.

Given the task, outline:
1. What files need to be modified
2. What specific changes are needed
3. How to verify the changes work

Then say "Ready to refactor."
Do not execute commands yet - just plan.
]],

    explorer = [[
You are a REFACTORER. Explore code, then make changes.

FORMAT: Commands MUST use $(cmd args) syntax exactly. No markdown.
CORRECT: $(view file.rs) $(edit file.rs/function_name replace new_code)
WRONG: ```$(edit ...)``` or `edit ...`

Exploration (prefer over shell):
$(view path) - examine code structure
$(text-search "pattern") - find specific patterns
$(analyze callers symbol) - find callers before changing
$(analyze callees symbol) - find callees

Editing (moss tools):
$(edit path/Symbol delete) - delete symbol
$(edit path/Symbol replace new_code) - replace symbol
$(edit path/Symbol insert --before code) - insert before
$(edit path/Symbol insert --after code) - insert after

Validation:
$(run cargo check) - check compilation
$(run cargo test) - run tests

Make ONE change at a time, then validate before continuing.
]],

    evaluator = [[
You are a REFACTOR EVALUATOR. Assess the changes made.

FORBIDDEN:
- $(view ...), $(text-search ...), $(edit ...) - you don't act

YOU MUST USE AT LEAST ONE:
- $(answer summary) - when refactoring complete
- $(note what was changed) - record progress
- $(keep 1 2) - retain validation output
- $(drop id) - remove stale items

Without one of these, context is lost and we loop forever.

Check: Did build pass? Tests pass? If failed, $(note failure) $(keep 1).
If complete: $(answer ## Changes\n- file: what changed\nValidation: passed)
]],
}

-- Role registry
M.ROLE_PROMPTS = {
    investigator = M.INVESTIGATOR_PROMPTS,
    auditor = M.AUDITOR_PROMPTS,
    refactorer = M.REFACTORER_PROMPTS,
}

-- LLM-based auto-dispatch classifier prompt
M.CLASSIFIER_PROMPT = [[
Classify this task into exactly one role. Reply with ONLY the role name.

Roles:
- investigator: questions, exploration, understanding code ("how does X work?", "where is Y?")
- auditor: finding issues, security/quality checks ("find bugs", "check for vulnerabilities")
- refactorer: making changes, fixing, updating code ("rename X to Y", "add error handling")

Task: %s

Role:]]

-- Build machine config for a given role
function M.build_machine(role)
    local prompts = M.ROLE_PROMPTS[role] or M.ROLE_PROMPTS.investigator
    return {
        start = "explorer",  -- can be overridden to "planner"

        states = {
            planner = {
                prompt = prompts.planner,
                context = "task_only",
                next = "explorer",
            },

            explorer = {
                prompt = prompts.explorer,
                context = "last_outputs",
                next = "evaluator",
            },

            evaluator = {
                prompt = prompts.evaluator,
                context = "working_memory",
                next = "explorer",
            },
        },
    }
end

-- Classify task into a role using LLM
function M.classify_task(task, provider, model)
    provider = provider or "gemini"
    local prompt = string.format(M.CLASSIFIER_PROMPT, task)

    -- Lightweight call with no history
    local response = llm.chat(provider, model, "", prompt, {})
    if not response then
        return "investigator"  -- fallback
    end

    -- Extract role from response
    local role_lower = response:lower():gsub("%s+", "")
    if role_lower:find("auditor") then
        return "auditor"
    elseif role_lower:find("refactor") then
        return "refactorer"
    else
        return "investigator"
    end
end

-- List available roles with descriptions
function M.list_roles()
    return {
        { name = "investigator", description = "Answers questions about the codebase" },
        { name = "auditor", description = "Finds issues (security, quality, patterns)" },
        { name = "refactorer", description = "Makes changes to fix issues" },
    }
end

return M
