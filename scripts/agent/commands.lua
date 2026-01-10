-- Agent command execution utilities
-- Usage: local commands = require("agent.commands")

local parser = require("agent.parser")

local M = {}

-- Execute batch edit from agent command string
-- Format: "target1 action1 content1 | target2 action2 content2"
-- Actions: delete, replace, insert
function M.execute_batch_edit(edits_str)
    local edits = {}
    local outputs = {}

    -- Split by | and parse each edit
    for edit_part in (edits_str .. "|"):gmatch("(.-)%s*|%s*") do
        edit_part = edit_part:match("^%s*(.-)%s*$")  -- trim
        if edit_part ~= "" then
            -- Parse: target action [content]
            local target, rest = edit_part:match("^(%S+)%s+(.+)$")
            if target then
                local action, content = rest:match("^(%S+)%s*(.*)$")
                if action then
                    local edit_obj = { target = target, action = action }
                    if content and content ~= "" then
                        edit_obj.content = content
                    end
                    table.insert(edits, edit_obj)
                end
            end
        end
    end

    if #edits == 0 then
        return { output = "No valid edits parsed", success = false }
    end

    -- Try to use edit.batch if available (Lua runtime context)
    if edit and edit.batch then
        local ok, result = pcall(function()
            return edit.batch(edits, { message = "Agent batch edit" })
        end)
        if ok and result then
            if result.success then
                table.insert(outputs, string.format("Batch edit applied: %d edits", result.edits_applied or #edits))
                if result.files_modified then
                    for _, f in ipairs(result.files_modified) do
                        table.insert(outputs, "  Modified: " .. f)
                    end
                end
                return { output = table.concat(outputs, "\n"), success = true }
            else
                return { output = result.error or "Batch edit failed", success = false }
            end
        end
    end

    -- Fallback: generate JSON and pipe to moss edit --batch -
    local json_edits = {}
    for _, e in ipairs(edits) do
        local json_edit = string.format('{"target": %s, "action": "%s"',
            parser.json_encode_string(e.target), e.action)
        if e.content then
            json_edit = json_edit .. string.format(', "content": %s', parser.json_encode_string(e.content))
        end
        json_edit = json_edit .. "}"
        table.insert(json_edits, json_edit)
    end
    local json = '{"edits": [' .. table.concat(json_edits, ", ") .. ']}'

    -- Execute via moss CLI
    local cmd = string.format("echo '%s' | %s edit --batch -", json:gsub("'", "'\\''"), _moss_bin)
    return shell(cmd)
end

return M
