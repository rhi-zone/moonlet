-- Agent session management (checkpoints, logs)
-- Usage: local session = require("agent.session")

local parser = require("agent.parser")

local M = {}

-- Characters for random IDs (no i,l,o,1,0 to avoid visual confusion)
local ID_CHARS = "abcdefghjkmnpqrstuvwxyz23456789"
local ID_LEN = 4
local SESSION_ID_LEN = 8

-- Generate random short IDs for memory items
function M.gen_id()
    local id = ""
    for _ = 1, ID_LEN do
        local idx = math.random(1, #ID_CHARS)
        id = id .. ID_CHARS:sub(idx, idx)
    end
    return id
end

-- Generate session ID
function M.gen_session_id()
    local id = ""
    for _ = 1, SESSION_ID_LEN do
        local idx = math.random(1, #ID_CHARS)
        id = id .. ID_CHARS:sub(idx, idx)
    end
    return id
end

-- Session checkpoint directory
local function get_session_dir()
    return _moss_root .. "/.moss/agent"
end

-- Session log directory
local function get_log_dir()
    return _moss_root .. "/.moss/agent/logs"
end

-- Format a log entry as JSON
function M.json_log_entry(event, data)
    local parts = {"{"}
    table.insert(parts, string.format('"event": "%s",', event))
    table.insert(parts, string.format('"timestamp": "%s"', os.date("!%Y-%m-%dT%H:%M:%SZ")))

    if data then
        for key, value in pairs(data) do
            if type(value) == "string" then
                table.insert(parts, string.format(', "%s": %s', key, parser.json_encode_string(value)))
            elseif type(value) == "number" then
                table.insert(parts, string.format(', "%s": %s', key, tostring(value)))
            elseif type(value) == "boolean" then
                table.insert(parts, string.format(', "%s": %s', key, tostring(value)))
            elseif type(value) == "table" then
                local json_arr = {}
                for _, v in ipairs(value) do
                    if type(v) == "string" then
                        table.insert(json_arr, parser.json_encode_string(v))
                    else
                        table.insert(json_arr, tostring(v))
                    end
                end
                table.insert(parts, string.format(', "%s": [%s]', key, table.concat(json_arr, ", ")))
            end
        end
    end

    table.insert(parts, "}")
    return table.concat(parts, "")
end

-- Start recording a session log
-- Returns a logger object with :log(event, data) method
function M.start_session_log(session_id)
    local log_dir = get_log_dir()
    os.execute("mkdir -p " .. log_dir)

    local log_file_path = log_dir .. "/" .. session_id .. ".jsonl"
    local log_file = io.open(log_file_path, "w")
    if not log_file then
        return nil
    end

    local logger = {
        file = log_file,
        session_id = session_id,
        start_time = os.time(),
        turn_count = 0
    }

    -- Write session start event
    logger.file:write(M.json_log_entry("session_start", {
        session_id = session_id,
        timestamp = os.date("!%Y-%m-%dT%H:%M:%SZ"),
        moss_root = _moss_root
    }) .. "\n")
    logger.file:flush()

    function logger:log(event, data)
        self.file:write(M.json_log_entry(event, data) .. "\n")
        self.file:flush()
    end

    function logger:close()
        self:log("session_end", {
            duration_seconds = os.time() - self.start_time,
            total_turns = self.turn_count
        })
        self.file:close()
    end

    return logger
end

-- List available session logs
function M.list_logs()
    local logs = {}
    local log_dir = get_log_dir()
    local handle = io.popen("ls -t " .. log_dir .. "/*.jsonl 2>/dev/null")
    if handle then
        for line in handle:lines() do
            local id = line:match("/([^/]+)%.jsonl$")
            if id then
                table.insert(logs, id)
            end
        end
        handle:close()
    end
    return logs
end

-- Save session checkpoint to JSON file
-- Returns: session_id on success, nil + error on failure
function M.save_checkpoint(session_id, state)
    local session_dir = get_session_dir()
    os.execute("mkdir -p " .. session_dir)

    local checkpoint_file = session_dir .. "/session-" .. session_id .. ".json"
    local file, err = io.open(checkpoint_file, "w")
    if not file then
        return nil, err
    end

    local json_parts = {"{"}
    table.insert(json_parts, string.format('"session_id": "%s",', session_id))
    table.insert(json_parts, string.format('"task": %s,', parser.json_encode_string(state.task)))
    table.insert(json_parts, string.format('"turn": %d,', state.turn))
    table.insert(json_parts, string.format('"timestamp": "%s",', os.date("%Y-%m-%dT%H:%M:%S")))

    -- Serialize working memory
    table.insert(json_parts, '"working_memory": [')
    for i, item in ipairs(state.working_memory) do
        local item_json = string.format(
            '{"type": "%s", "id": "%s", "content": %s%s}',
            item.type,
            item.id,
            parser.json_encode_string(item.content),
            item.cmd and string.format(', "cmd": %s, "success": %s', parser.json_encode_string(item.cmd), tostring(item.success)) or ""
        )
        table.insert(json_parts, item_json)
        if i < #state.working_memory then
            table.insert(json_parts, ",")
        end
    end
    table.insert(json_parts, "],")

    -- Serialize progress summary
    table.insert(json_parts, string.format('"progress": %s,', parser.json_encode_string(state.progress or "")))
    table.insert(json_parts, string.format('"open_questions": %s', parser.json_encode_string(state.open_questions or "")))
    table.insert(json_parts, "}")

    file:write(table.concat(json_parts, "\n"))
    file:close()

    return session_id
end

-- Load session checkpoint from JSON file
-- Returns: state table on success, nil + error on failure
function M.load_checkpoint(session_id)
    local checkpoint_file = get_session_dir() .. "/session-" .. session_id .. ".json"
    local file, err = io.open(checkpoint_file, "r")
    if not file then
        return nil, "Session not found: " .. session_id
    end

    local content = file:read("*a")
    file:close()

    local state = M.parse_checkpoint_json(content)
    if not state then
        return nil, "Failed to parse checkpoint"
    end

    return state
end

-- Simple JSON parser for checkpoint format
function M.parse_checkpoint_json(json)
    local state = {working_memory = {}}

    state.session_id = json:match('"session_id":%s*"([^"]*)"')
    state.task = parser.json_decode_string(json:match('"task":%s*(".-[^\\]")'))
    state.turn = tonumber(json:match('"turn":%s*(%d+)'))
    state.progress = parser.json_decode_string(json:match('"progress":%s*(".-[^\\]")'))
    state.open_questions = parser.json_decode_string(json:match('"open_questions":%s*(".-[^\\]")'))

    local wm_json = json:match('"working_memory":%s*%[(.-)%]')
    if wm_json then
        for item_json in wm_json:gmatch('{([^}]+)}') do
            local item = {}
            item.type = item_json:match('"type":%s*"([^"]*)"')
            item.id = item_json:match('"id":%s*"([^"]*)"')
            item.content = parser.json_decode_string(item_json:match('"content":%s*(".-[^\\]")'))
            local cmd = item_json:match('"cmd":%s*(".-[^\\]")')
            if cmd then
                item.cmd = parser.json_decode_string(cmd)
                item.success = item_json:match('"success":%s*true') ~= nil
            end
            table.insert(state.working_memory, item)
        end
    end

    return state
end

-- List available session checkpoints
function M.list_sessions()
    local sessions = {}
    local session_dir = get_session_dir()
    local handle = io.popen("ls -t " .. session_dir .. "/session-*.json 2>/dev/null")
    if handle then
        for line in handle:lines() do
            local id = line:match("session%-(.+)%.json$")
            if id then
                table.insert(sessions, id)
            end
        end
        handle:close()
    end
    return sessions
end

-- Memorize a fact to long-term memory (.moss/memory/facts.md)
-- Returns: true on success, false + error message on failure
function M.memorize(fact)
    local memory_dir = _moss_root .. "/.moss/memory"
    local facts_file = memory_dir .. "/facts.md"

    -- Ensure directory exists
    os.execute("mkdir -p " .. memory_dir)

    -- Append fact with timestamp
    local file, err = io.open(facts_file, "a")
    if not file then
        return false, err
    end

    local timestamp = os.date("%Y-%m-%d %H:%M")
    file:write("- " .. fact .. " (" .. timestamp .. ")\n")
    file:close()

    return true
end

return M
