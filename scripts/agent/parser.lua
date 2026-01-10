-- Agent parsing utilities
-- Usage: local parser = require("agent.parser")

local M = {}

-- JSON string encoding
function M.json_encode_string(s)
    if s == nil then return "null" end
    s = tostring(s)
    s = s:gsub('\\', '\\\\')
    s = s:gsub('"', '\\"')
    s = s:gsub('\n', '\\n')
    s = s:gsub('\r', '\\r')
    s = s:gsub('\t', '\\t')
    return '"' .. s .. '"'
end

-- JSON string decoding
function M.json_decode_string(s)
    if not s or s == "null" then return nil end
    s = s:match('^"(.*)"$') or s
    s = s:gsub('\\n', '\n')
    s = s:gsub('\\r', '\r')
    s = s:gsub('\\t', '\t')
    s = s:gsub('\\"', '"')
    s = s:gsub('\\\\', '\\')
    return s
end

-- Parse $(command args) from LLM response
-- Returns: { {name = "cmd", args = "...", full = "cmd ..."}, ... }
function M.parse_commands(response)
    local commands = {}
    local i = 1
    local len = #response

    while i <= len do
        -- Look for $(
        local start = response:find("%$%(", i)
        if not start then break end

        -- Find matching ) considering quotes
        local j = start + 2  -- skip $(
        local in_quote = nil  -- nil, '"', or "'"
        local depth = 1

        while j <= len and depth > 0 do
            local c = response:sub(j, j)

            if in_quote then
                -- Inside quotes: only look for matching quote (handle escapes)
                if c == in_quote and response:sub(j - 1, j - 1) ~= '\\' then
                    in_quote = nil
                end
            else
                -- Outside quotes
                if c == '"' or c == "'" then
                    in_quote = c
                elseif c == '(' then
                    depth = depth + 1
                elseif c == ')' then
                    depth = depth - 1
                end
            end
            j = j + 1
        end

        if depth == 0 then
            -- Extract content between $( and )
            local content = response:sub(start + 2, j - 2)
            local cmd_name, args = content:match('^(%S+)%s*(.*)$')
            if cmd_name then
                table.insert(commands, {
                    name = cmd_name,
                    args = args or "",
                    full = cmd_name .. " " .. (args or "")
                })
            end
        end

        i = j
    end

    return commands
end

-- Parse keep command: "keep" | "keep all" | "keep 1" | "keep 1 2 3"
-- Returns indices of outputs to keep
function M.parse_keep(cmd, num_outputs)
    local indices = {}
    local args = cmd:match("^keep%s*(.*)")
    if not args or args == "" or args == "all" then
        for i = 1, num_outputs do
            table.insert(indices, i)
        end
    else
        for idx in args:gmatch("%d+") do
            local n = tonumber(idx)
            if n and n >= 1 and n <= num_outputs then
                table.insert(indices, n)
            end
        end
    end
    return indices
end

-- Parse CLI arguments
-- Returns options table with parsed flags and prompt
function M.parse_args(args)
    local opts = {}
    local task_parts = {}
    local i = 1
    while i <= #args do
        local arg = args[i]
        if arg == "--help" or arg == "-h" then
            opts.help = true
            i = i + 1
        elseif arg == "--provider" and args[i+1] then
            opts.provider = args[i+1]
            i = i + 2
        elseif arg == "--model" and args[i+1] then
            opts.model = args[i+1]
            i = i + 2
        elseif arg == "--max-turns" and args[i+1] then
            opts.max_turns = tonumber(args[i+1])
            i = i + 2
        elseif arg == "--explain" then
            opts.explain = true
            i = i + 1
        elseif arg == "--resume" and args[i+1] then
            opts.resume = args[i+1]
            i = i + 2
        elseif arg == "--list-sessions" then
            opts.list_sessions = true
            i = i + 1
        elseif arg == "--list-logs" then
            opts.list_logs = true
            i = i + 1
        elseif arg == "--non-interactive" or arg == "-n" then
            opts.non_interactive = true
            i = i + 1
        elseif arg == "--plan" then
            opts.plan = true
            i = i + 1
        elseif arg == "--role" and args[i+1] then
            opts.role = args[i+1]
            i = i + 2
        elseif arg == "--audit" then
            opts.role = "auditor"
            i = i + 1
        elseif arg == "--refactor" then
            opts.role = "refactorer"
            opts.plan = true  -- refactorer should always plan first
            i = i + 1
        elseif arg == "--validate" and args[i+1] then
            opts.validate_cmd = args[i+1]
            i = i + 2
        elseif arg == "--shadow" then
            opts.shadow = true
            i = i + 1
        elseif arg == "--auto-validate" then
            opts.auto_validate = true
            i = i + 1
        elseif arg == "--auto-approve" then
            if args[i+1] and not args[i+1]:match("^%-") then
                opts.auto_approve = args[i+1]
                i = i + 2
            else
                opts.auto_approve = "low"
                i = i + 1
            end
        elseif arg == "--commit" then
            opts.commit = true
            i = i + 1
        elseif arg == "--retry-on-failure" then
            if args[i+1] and args[i+1]:match("^%d+$") then
                opts.retry_on_failure = tonumber(args[i+1])
                i = i + 2
            else
                opts.retry_on_failure = 1
                i = i + 1
            end
        elseif arg == "--diff" then
            -- Optional base ref, default to auto-detect
            if args[i+1] and not args[i+1]:match("^%-") then
                opts.diff_base = args[i+1]
                i = i + 2
            else
                opts.diff_base = ""  -- empty means auto-detect
                i = i + 1
            end
        elseif arg == "--auto" then
            opts.auto_dispatch = true
            i = i + 1
        elseif arg == "--roles" then
            opts.list_roles = true
            i = i + 1
        else
            table.insert(task_parts, arg)
            i = i + 1
        end
    end
    opts.prompt = table.concat(task_parts, " ")
    if opts.prompt == "" then opts.prompt = nil end
    return opts
end

return M
