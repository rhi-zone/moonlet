-- Agent context building functions
-- Usage: local context = require("agent.context")

local M = {}

-- Build context for planner state (task only)
function M.build_planner_context(task)
    return "**Task:** " .. task .. "\n\nCreate a plan to accomplish this task."
end

-- Build context for explorer state (ephemeral - last outputs only)
function M.build_explorer_context(task, last_outputs, notes, plan)
    local parts = {"**Task:** " .. task}

    if plan then
        table.insert(parts, "\n**Plan:**\n" .. plan)
    end

    if #notes > 0 then
        table.insert(parts, "\n**Notes so far:**")
        for _, note in ipairs(notes) do
            table.insert(parts, "- " .. note)
        end
    end

    if last_outputs and #last_outputs > 0 then
        table.insert(parts, "\n**Last results:**")
        for _, out in ipairs(last_outputs) do
            local status = out.success and "" or " (failed)"
            table.insert(parts, string.format("\n`%s`%s\n```\n%s\n```", out.cmd, status, out.content))
        end
    end

    table.insert(parts, "\nSuggest commands to explore. Do not conclude yet.")
    return table.concat(parts, "\n")
end

-- Build context for evaluator state (working memory + pending outputs)
function M.build_evaluator_context(task, working_memory, last_outputs, notes)
    local parts = {"**Task:** " .. task}

    -- Check for failures in new results
    local has_failures = false
    for _, out in ipairs(last_outputs) do
        if not out.success then
            has_failures = true
            break
        end
    end

    if has_failures then
        table.insert(parts, "\n**WARNING: Some commands failed.** Consider alternative approaches.")
    end

    if #notes > 0 then
        table.insert(parts, "\n**Notes:**")
        for _, note in ipairs(notes) do
            table.insert(parts, "- " .. note)
        end
    end

    if #working_memory > 0 then
        table.insert(parts, "\n**Working memory** (kept from previous turns):")
        for i, item in ipairs(working_memory) do
            local status = item.success and "" or " (FAILED)"
            table.insert(parts, string.format("\n[%d] `%s`%s\n```\n%s\n```", i, item.cmd, status, item.content))
        end
    end

    if #last_outputs > 0 then
        table.insert(parts, "\n**New results** (will be discarded unless you keep them):")
        for i, out in ipairs(last_outputs) do
            local status = out.success and "" or " (FAILED)"
            table.insert(parts, string.format("\n[%d] `%s`%s\n```\n%s\n```", i, out.cmd, status, out.content))
        end
    end

    return table.concat(parts, "\n")
end

-- Build error escalation context
-- error_state: {cmd, retries, rolled_back, last_error}
function M.build_error_context(error_state)
    if not error_state then return "" end

    local parts = {"\n[error recovery]"}
    table.insert(parts, string.format("Command '%s' has failed %d time(s).", error_state.cmd, error_state.retries))

    if error_state.last_error then
        table.insert(parts, "Last error: " .. error_state.last_error:sub(1, 200))
    end

    if error_state.rolled_back then
        table.insert(parts, "Status: Already rolled back to pre-edit state.")
        table.insert(parts, "Options:")
        table.insert(parts, "  1. $(ask <question>) - Get user guidance")
        table.insert(parts, "  2. Try a different approach")
        table.insert(parts, "  3. $(done BLOCKED: <explanation>) - Give up with explanation")
    elseif error_state.retries >= 3 then
        table.insert(parts, "Status: Max retries reached, will rollback on next failure.")
        table.insert(parts, "Consider: Try a different approach or $(ask) for help.")
    else
        table.insert(parts, string.format("Status: Retry %d/3", error_state.retries))
    end

    table.insert(parts, "[/error recovery]")
    return table.concat(parts, "\n")
end

-- Build context from working memory (for main agent loop)
-- working_memory: list of {type="output"|"note", cmd?, content, id}
-- error_state: optional {cmd, retries, rolled_back, last_error}
function M.build_context(task, working_memory, current_outputs, error_state)
    local parts = {"**Task:** " .. task}

    -- Add error recovery context if in error state
    if error_state then
        table.insert(parts, M.build_error_context(error_state))
    end

    -- Add working memory (kept outputs and notes) - markdown format
    if #working_memory > 0 then
        table.insert(parts, "\n**Saved:**")
        for _, item in ipairs(working_memory) do
            if item.type == "note" then
                table.insert(parts, string.format("- [%s] %s", item.id, item.content))
            else
                local status = item.success and "" or " (failed)"
                table.insert(parts, string.format("\n`[%s] %s`%s\n```\n%s\n```", item.id, item.cmd, status, item.content))
            end
        end
    end

    -- Add current turn outputs (ephemeral) - markdown format
    if current_outputs and #current_outputs > 0 then
        table.insert(parts, "\n**Results:**")
        for i, out in ipairs(current_outputs) do
            local status = out.success and "" or " (failed)"
            table.insert(parts, string.format("\n`[%d] %s`%s\n```\n%s\n```", i, out.cmd, status, out.content))
        end
        table.insert(parts, "\n*Results disappear next turn.* Use $(keep), $(note), or $(done ANSWER).")
    end

    return table.concat(parts, "\n")
end

return M
