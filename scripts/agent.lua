-- Agent module: autonomous task execution with moss tools
local M = {}

-- Submodules
local risk = require("agent.risk")
local parser = require("agent.parser")
local session = require("agent.session")
local context = require("agent.context")
local commands = require("agent.commands")
local roles = require("agent.roles")

-- Seed random on load
math.randomseed(os.time())

-- ID generation (delegated to agent.session module)
M.gen_id = session.gen_id
M.gen_session_id = session.gen_session_id

-- Risk assessment (delegated to agent.risk module)
M.RISK = risk.RISK
M.assess_risk = risk.assess_risk
M.should_auto_approve = risk.should_auto_approve
M.detect_validator = risk.detect_validator

-- Session management (delegated to agent.session module)
M.start_session_log = session.start_session_log
M.json_log_entry = session.json_log_entry
M.list_logs = session.list_logs
M.save_checkpoint = session.save_checkpoint
M.load_checkpoint = session.load_checkpoint
M.parse_checkpoint_json = session.parse_checkpoint_json
M.list_sessions = session.list_sessions

-- JSON utilities (delegated to agent.parser module)
M.json_encode_string = parser.json_encode_string
M.json_decode_string = parser.json_decode_string

-- Long-term memory (delegated to agent.session module)
M.memorize = session.memorize

-- Batch edit execution (delegated to agent.commands module)
M.execute_batch_edit = commands.execute_batch_edit

-- Role prompts and state machine config (delegated to agent.roles module)
M.classify_task = roles.classify_task
local build_machine = roles.build_machine

-- Default machine (for backwards compat)
local MACHINE = build_machine("investigator")

-- Context building (delegated to agent.context module)
M.build_planner_context = context.build_planner_context
M.build_explorer_context = context.build_explorer_context
M.build_evaluator_context = context.build_evaluator_context

-- State machine agent runner (v2)
function M.run_state_machine(opts)
    opts = opts or {}
    local task = opts.prompt or opts.task or "Help with this codebase"
    local max_turns = opts.max_turns or 10
    local provider = opts.provider or "gemini"
    local model = opts.model  -- nil means use provider default
    local use_planner = opts.plan or false
    local role = opts.role
    if not role then
        if opts.auto_dispatch and task then
            print("[agent] Classifying task...")
            role = M.classify_task(task, provider, model)
            print(string.format("[agent] Auto-dispatch → %s", role))
        else
            role = "investigator"
        end
    end

    -- Refactorer always plans first
    if role == "refactorer" then
        use_planner = true
    end

    -- Initialize shadow worktree for safe editing (--shadow flag or auto for refactorer)
    local shadow_enabled = opts.shadow
    if shadow_enabled then
        print("[agent] Initializing shadow worktree for safe editing...")
        local ok, err = pcall(function()
            shadow.worktree.open()
            shadow.worktree.sync()
            shadow.worktree.enable()
        end)
        if ok then
            print("[agent] Shadow mode enabled - edits go to .moss/shadow/worktree/")
        else
            print("[agent] Warning: Failed to initialize shadow worktree: " .. tostring(err))
            shadow_enabled = false
        end
    end

    -- Auto-detect validator if:
    -- 1. Shadow enabled and no explicit --validate, OR
    -- 2. --auto-validate flag is set
    if (shadow_enabled or opts.auto_validate) and not opts.validate_cmd then
        local detected_cmd, detected_type = M.detect_validator()
        if detected_cmd then
            opts.validate_cmd = detected_cmd
            print("[agent] Auto-detected validator: " .. detected_cmd .. " (" .. detected_type .. ")")
        end
    end

    -- Handle --diff: get changed files and add to task context
    if opts.diff_base ~= nil then
        local base = opts.diff_base
        if base == "" then
            local detect = shell("git symbolic-ref refs/remotes/origin/HEAD 2>/dev/null || git rev-parse --verify origin/main 2>/dev/null || git rev-parse --verify origin/master 2>/dev/null || git rev-parse --verify main 2>/dev/null || git rev-parse --verify master 2>/dev/null")
            if detect.success and detect.output and detect.output ~= "" then
                base = detect.output:match("refs/remotes/(.+)") or detect.output:gsub("%s+", "")
            else
                base = "HEAD~10"
            end
        end

        local merge_base_result = shell("git merge-base " .. base .. " HEAD 2>/dev/null")
        local effective_base = merge_base_result.success and merge_base_result.output:gsub("%s+", "") or base

        local diff_result = shell("git diff --name-only " .. effective_base)
        if diff_result.success and diff_result.output and diff_result.output ~= "" then
            local diff_files = {}
            for file in diff_result.output:gmatch("[^\n]+") do
                table.insert(diff_files, file)
            end
            print("[agent] Focusing on " .. #diff_files .. " changed files (vs " .. base .. ")")
            task = task .. "\n\nFOCUS: Only analyze these changed files:\n"
            for _, f in ipairs(diff_files) do
                task = task .. "  - " .. f .. "\n"
            end
            task = task .. "\nIgnore unchanged files unless directly relevant to changes."
        end
    end

    -- Build machine config for this role
    local machine = build_machine(role)

    -- Session ID and resume state
    local session_id = opts.resume or M.gen_session_id()
    local start_turn = 0
    local resumed_state = nil
    local resumed_notes = nil
    local resumed_working_memory = nil
    local resumed_plan = nil

    -- Resume from checkpoint if specified
    if opts.resume then
        local checkpoint, err = M.load_checkpoint(opts.resume)
        if checkpoint then
            print("[agent] Resuming session: " .. opts.resume)
            task = checkpoint.task or task
            start_turn = checkpoint.turn or 0
            resumed_state = checkpoint.state
            resumed_notes = checkpoint.notes
            resumed_working_memory = checkpoint.working_memory
            resumed_plan = checkpoint.plan
            role = checkpoint.role or role
            if checkpoint.progress then
                print("[agent] Previous progress: " .. checkpoint.progress)
            end
            if checkpoint.open_questions then
                print("[agent] Open questions: " .. checkpoint.open_questions)
            end
        else
            print("[agent] Warning: " .. (err or "Failed to load checkpoint"))
            print("[agent] Starting fresh session")
            session_id = M.gen_session_id()
        end
    end

    print(string.format("[agent:%s] Session: %s", role, session_id))

    -- Start session logging
    local session_log = M.start_session_log(session_id)
    local start_state = resumed_state or (use_planner and "planner" or "explorer")
    if session_log then
        session_log:log("task", {
            system_prompt = "state_machine_v2",
            user_prompt = task,
            provider = provider,
            model = model or "default",
            max_turns = max_turns,
            machine_start = start_state,
            use_planner = use_planner,
            role = role
        })
    end

    local state = start_state
    local notes = resumed_notes or {}           -- accumulated notes
    local working_memory = resumed_working_memory or {}  -- curated outputs kept by evaluator
    local last_outputs = {}    -- most recent turn's outputs (pending curation)
    local plan = resumed_plan  -- plan from planner state
    local recent_cmds = {}     -- for loop detection
    local turn = start_turn
    local validation_retry_count = 0  -- track validation retries
    local evaluator_cycles = 0        -- track evaluator→explorer cycles without progress
    local max_evaluator_cycles = opts.max_cycles or 5  -- bail if stuck in explore/evaluate loop
    local non_interactive = opts.non_interactive or false

    while turn < max_turns do
        turn = turn + 1
        local state_config = machine.states[state]

        -- Build context based on state
        local context
        if state == "planner" then
            context = M.build_planner_context(task)
        elseif state == "explorer" then
            context = M.build_explorer_context(task, last_outputs, notes, plan)
        else
            context = M.build_evaluator_context(task, working_memory, last_outputs, notes)
        end

        print(string.format("[agent] Turn %d/%d (%s)", turn, max_turns, state))
        io.write("[agent] Thinking... ")
        io.flush()

        -- Log turn start
        if session_log then
            session_log.turn_count = turn
            session_log:log("turn_start", {
                turn = turn,
                state = state,
                working_memory_count = #working_memory,
                notes_count = #notes,
                pending_outputs = #last_outputs
            })
        end

        -- LLM call
        local response = llm.chat(provider, model, state_config.prompt, context, {})
        io.write("done\n")
        print(response)

        -- Log LLM response
        if session_log then
            session_log:log("llm_response", {
                turn = turn,
                state = state,
                response = response:sub(1, 500)  -- truncate for log
            })
        end

        -- Handle planner state - save plan and transition
        if state == "planner" then
            plan = response
            if session_log then
                session_log:log("plan_created", { plan = response:sub(1, 500) })
            end
            state = state_config.next
            goto continue
        end

        -- Parse commands from response (handles quoted strings properly)
        local commands = M.parse_commands(response)

        -- Handle $(done) or $(answer) - only valid in evaluator state
        for _, cmd in ipairs(commands) do
            if cmd.name == "done" or cmd.name == "answer" then
                if state == "evaluator" then
                    local final_answer = cmd.args
                    -- Models often output "$(done ANSWER) - actual answer"
                    -- If args is just "ANSWER", look for text after the $(done ...) in response
                    if final_answer == "ANSWER" then
                        local after = response:match('%$%(done%s+ANSWER%)%s*[-:]?%s*(.+)')
                        if after then
                            final_answer = after:gsub('\n.*', '')  -- first line only
                        end
                    end
                    -- Handle shadow mode finalization
                    if shadow_enabled then
                        print("[agent] Finalizing shadow edits...")
                        local diff = shadow.worktree.diff()
                        if diff and #diff > 0 then
                            print("[agent] Changes in shadow worktree:")
                            print(diff)

                            -- Validate if validate_cmd is set
                            local should_apply = true
                            local validation_error = nil
                            if opts.validate_cmd then
                                print("[agent] Validating: " .. opts.validate_cmd)
                                local validation = shadow.worktree.validate(opts.validate_cmd)
                                if validation.success then
                                    print("[agent] Validation passed ✓")
                                else
                                    print("[agent] Validation FAILED:")
                                    validation_error = validation.stdout or validation.stderr or "Unknown error"
                                    print(validation_error)
                                    should_apply = false
                                end
                            end

                            if should_apply then
                                print("[agent] Applying shadow changes to real repo...")
                                local applied = shadow.worktree.apply()
                                print("[agent] Applied " .. #applied .. " file(s)")

                                -- Auto-commit if --commit flag is set
                                if opts.commit and #applied > 0 then
                                    print("[agent] Creating git commit...")
                                    -- Stage all applied files
                                    for _, file in ipairs(applied) do
                                        shell("git add " .. file)
                                    end
                                    -- Generate commit message from task
                                    local commit_msg = task:sub(1, 50)
                                    if #task > 50 then
                                        commit_msg = commit_msg .. "..."
                                    end
                                    local result = shell("git commit -m '[moss agent] " .. commit_msg:gsub("'", "'\\''") .. "'")
                                    if result.success then
                                        print("[agent] Committed changes ✓")
                                    else
                                        print("[agent] Warning: git commit failed - " .. (result.output or ""))
                                    end
                                end
                            else
                                -- Validation failed - retry if allowed
                                local max_retries = opts.retry_on_failure or 0
                                if validation_retry_count < max_retries then
                                    validation_retry_count = validation_retry_count + 1
                                    print("[agent] Retrying (" .. validation_retry_count .. "/" .. max_retries .. ")...")
                                    -- Reset shadow and inject error into working memory
                                    shadow.worktree.reset()
                                    shadow.worktree.sync()  -- Resync to clean state
                                    table.insert(working_memory, {
                                        id = M.gen_id(),
                                        type = "error",
                                        content = "VALIDATION FAILED. Please fix this error and try again:\n" .. validation_error,
                                    })
                                    -- Don't return - continue the state machine
                                    goto continue
                                else
                                    print("[agent] Discarding shadow changes (validation failed" ..
                                        (max_retries > 0 and ", max retries reached" or "") .. ")")
                                    shadow.worktree.reset()
                                end
                            end
                        else
                            print("[agent] No shadow changes to apply")
                        end
                        shadow.worktree.disable()
                    end

                    print("[agent] Answer: " .. final_answer)
                    if session_log then
                        session_log:log("done", { answer = final_answer:sub(1, 200), turn = turn })
                        session_log:close()
                    end
                    return {success = true, answer = final_answer, turns = turn}
                else
                    print("[agent] Warning: $(" .. cmd.name .. ") ignored in explorer state")
                end
            end
        end

        -- Handle $(note) commands
        for _, cmd in ipairs(commands) do
            if cmd.name == "note" then
                table.insert(notes, cmd.args)
                print("[agent] Noted: " .. cmd.args)
            end
        end

        -- Handle $(checkpoint) command - saves session state and exits
        for _, cmd in ipairs(commands) do
            if cmd.name == "checkpoint" then
                local args = cmd.args or ""
                local progress, open_questions = "", ""
                -- Parse "progress | open questions" format
                local p, q = args:match("^(.-)%s*|%s*(.*)$")
                if p then
                    progress = p
                    open_questions = q
                else
                    progress = args
                end

                local checkpoint_state = {
                    task = task,
                    turn = turn,
                    state = state,
                    notes = notes,
                    working_memory = working_memory,
                    plan = plan,
                    role = role,
                    progress = progress,
                    open_questions = open_questions
                }

                local saved_id, err = M.save_checkpoint(session_id, checkpoint_state)
                if saved_id then
                    print("[agent] Session checkpointed: " .. saved_id)
                    print("[agent] Resume with: moss @agent --resume " .. saved_id)
                    if progress ~= "" then
                        print("[agent] Progress: " .. progress)
                    end
                    if open_questions ~= "" then
                        print("[agent] Open questions: " .. open_questions)
                    end
                else
                    print("[agent] Failed to checkpoint: " .. (err or "unknown error"))
                end

                if shadow_enabled then
                    shadow.worktree.disable()
                end
                if session_log then
                    session_log:log("checkpoint", { progress = progress, open_questions = open_questions })
                    session_log:close()
                end
                return {success = true, checkpointed = true, session_id = session_id, turns = turn}
            end
        end

        -- Handle $(keep) and $(drop) - only in evaluator state
        if state == "evaluator" then
            for _, cmd in ipairs(commands) do
                if cmd.name == "keep" then
                    local indices = M.parse_keep("keep " .. cmd.args, #last_outputs)
                    for _, idx in ipairs(indices) do
                        if last_outputs[idx] then
                            table.insert(working_memory, last_outputs[idx])
                            print("[agent] Kept: " .. last_outputs[idx].cmd)
                        end
                    end
                elseif cmd.name == "drop" then
                    local idx = tonumber(cmd.args)
                    if idx and working_memory[idx] then
                        print("[agent] Dropped: " .. working_memory[idx].cmd)
                        table.remove(working_memory, idx)
                    end
                end
            end
        end

        -- Execute exploration commands (only in explorer state)
        if state == "explorer" then
            last_outputs = {}
            for _, cmd in ipairs(commands) do
                if cmd.name ~= "note" and cmd.name ~= "done" and cmd.name ~= "answer" and cmd.name ~= "checkpoint" then
                    local result
                    if cmd.name == "ask" then
                        -- Handle $(ask) command - request user input
                        if non_interactive then
                            print("[agent] BLOCKED: " .. cmd.args .. " (non-interactive mode)")
                            result = { output = "ERROR: Cannot ask user in non-interactive mode. Question was: " .. cmd.args, success = false }
                            if session_log then
                                session_log:log("blocked_ask", { question = cmd.args, turn = turn })
                            end
                        else
                            io.write("[agent] " .. cmd.args .. "\n> ")
                            io.flush()
                            local answer = io.read("*l") or ""
                            result = { output = "User: " .. answer, success = true }
                        end
                    elseif cmd.name == "run" then
                        print("[agent] Running: " .. cmd.args)
                        result = shell(cmd.args)
                    elseif cmd.name == "view" or cmd.name == "text-search" or
                           cmd.name == "analyze" or cmd.name == "package" or
                           cmd.name == "edit" then
                        print("[agent] Running: " .. cmd.full)
                        result = shell(_moss_bin .. " " .. cmd.full)
                    else
                        -- Unknown command, skip
                        print("[agent] Skipping unknown: " .. cmd.name)
                        result = nil
                    end
                    if result then
                        -- Truncate large outputs to prevent context bloat
                        local content = result.output or ""
                        local MAX_OUTPUT = 10000  -- ~10KB per command output
                        local truncated = false
                        if #content > MAX_OUTPUT then
                            content = content:sub(1, MAX_OUTPUT)
                            content = content .. "\n... [OUTPUT TRUNCATED - " .. (#(result.output or "") - MAX_OUTPUT) .. " more bytes]\n"
                            truncated = true
                        end
                        table.insert(last_outputs, {
                            cmd = cmd.full,
                            content = content,
                            success = result.success,
                            truncated = truncated
                        })
                        if session_log then
                            session_log:log("command", {
                                cmd = cmd.full,
                                success = result.success,
                                output_length = (result.output or ""):len(),
                                turn = turn
                            })
                        end
                    end
                end
            end

            -- Track commands for loop detection
            for _, out in ipairs(last_outputs) do
                table.insert(recent_cmds, out.cmd)
            end

            -- Check for loops (same command 3+ times in a row)
            if M.is_looping(recent_cmds, 3) then
                print("[agent] Loop detected, auto-checkpointing...")
                local checkpoint_state = {
                    task = task,
                    turn = turn,
                    state = state,
                    notes = notes,
                    working_memory = working_memory,
                    plan = plan,
                    role = role,
                    progress = "Session ended due to loop detection",
                    open_questions = "Agent was repeating: " .. (recent_cmds[#recent_cmds] or "unknown")
                }
                local saved_id, _ = M.save_checkpoint(session_id, checkpoint_state)
                if saved_id then
                    print("[agent] Session checkpointed: " .. saved_id)
                    print("[agent] Resume with: moss @agent --resume " .. saved_id)
                end
                if shadow_enabled then
                    print("[agent] Discarding shadow changes (loop detected)")
                    shadow.worktree.reset()
                    shadow.worktree.disable()
                end
                if session_log then
                    session_log:log("loop_detected", { cmd = recent_cmds[#recent_cmds], turn = turn })
                    session_log:close()
                end
                return {success = false, reason = "loop_detected", turns = turn, session_id = session_id}
            end
        end

        -- Track evaluator→explorer cycles to detect infinite loops
        if state == "evaluator" and state_config.next == "explorer" then
            evaluator_cycles = evaluator_cycles + 1
            if evaluator_cycles >= max_evaluator_cycles then
                print(string.format("[agent] Evaluator cycle limit reached (%d), forcing conclusion", max_evaluator_cycles))
                -- Force the agent to conclude by switching to final evaluator prompt
                local forced_context = M.build_evaluator_context(task, working_memory, last_outputs, notes)
                forced_context = forced_context .. "\n\nIMPORTANT: You have explored enough. You MUST now provide $(done YOUR_ANSWER) with your best answer based on available information. Do not request more exploration."

                local forced_response = llm.complete(provider, model, state_config.prompt, forced_context)
                if forced_response then
                    local forced_cmds = M.parse_commands(forced_response)
                    for _, cmd in ipairs(forced_cmds) do
                        if cmd.name == "done" or cmd.name == "answer" then
                            local final_answer = cmd.args
                            if final_answer == "ANSWER" then
                                local after = forced_response:match('%$%(done%s+ANSWER%)%s*[-:]?%s*(.+)')
                                if after then final_answer = after:gsub('\n.*', '') end
                            end
                            print("[agent] Forced answer: " .. final_answer)
                            if session_log then
                                session_log:log("done", { answer = final_answer:sub(1, 200), turn = turn, forced = true })
                                session_log:close()
                            end
                            return {success = true, answer = final_answer, turns = turn, forced = true}
                        end
                    end
                end
                -- If still no answer, checkpoint and bail
                print("[agent] Could not force conclusion, checkpointing...")
                local checkpoint_state = {
                    task = task, turn = turn, state = state, notes = notes,
                    working_memory = working_memory, plan = plan, role = role,
                    progress = "Evaluator stuck in loop, could not conclude",
                    open_questions = "Task may be too complex - consider breaking it down"
                }
                local saved_id, _ = M.save_checkpoint(session_id, checkpoint_state)
                if saved_id then
                    print("[agent] Session checkpointed: " .. saved_id)
                    print("[agent] Resume with: moss @agent --resume " .. saved_id)
                end
                if session_log then
                    session_log:log("cycle_limit", { cycles = evaluator_cycles, turn = turn })
                    session_log:close()
                end
                return {success = false, reason = "cycle_limit", turns = turn, session_id = session_id}
            end
        elseif state == "explorer" then
            -- Reset cycle counter if we're making progress (new outputs)
            if #last_outputs > 0 then
                evaluator_cycles = 0
            end
        end

        -- Transition to next state
        state = state_config.next
        ::continue::
    end

    -- Auto-checkpoint on max turns reached
    print("[agent] Max turns reached, auto-checkpointing...")
    local checkpoint_state = {
        task = task,
        turn = turn,
        state = state,
        notes = notes,
        working_memory = working_memory,
        plan = plan,
        role = role,
        progress = "Session ended at max turns",
        open_questions = "Review working memory for context"
    }
    local saved_id, err = M.save_checkpoint(session_id, checkpoint_state)
    if saved_id then
        print("[agent] Session auto-checkpointed: " .. saved_id)
        print("[agent] Resume with: moss @agent --resume " .. saved_id)
    else
        print("[agent] Warning: Failed to auto-checkpoint: " .. (err or "unknown error"))
    end

    if shadow_enabled then
        print("[agent] Discarding shadow changes (max turns)")
        shadow.worktree.reset()
        shadow.worktree.disable()
    end
    if session_log then
        session_log:log("max_turns_reached", { turn = turn })
        session_log:close()
    end
    return {success = false, reason = "max_turns", turns = turn, session_id = session_id}
end

-- Check if last N commands are identical (loop detection)
-- recent_cmds is a list of recent command strings
function M.is_looping(recent_cmds, n)
    n = n or 3
    if #recent_cmds < n then return false end

    local last_cmd = recent_cmds[#recent_cmds]
    for i = 1, n - 1 do
        if recent_cmds[#recent_cmds - i] ~= last_cmd then
            return false
        end
    end
    return true
end

-- Context building continued (delegated to agent.context module)
M.build_error_context = context.build_error_context
M.build_context = context.build_context

-- Command parsing (delegated to agent.parser module)
M.parse_commands = parser.parse_commands
M.parse_keep = parser.parse_keep

-- Main agent loop
function M.show_help()
    print([[Usage: moss @agent [options] <task>

Options:
  --provider <name>   LLM provider (gemini, openrouter, ollama)
  --model <name>      Model name for the provider
  --max-turns <n>     Maximum conversation turns (default: 15)
  --explain           Trace: show full tool outputs
  --resume <id>       Resume from a previous session
  --list-sessions     List available sessions to resume
  --list-logs         List session log files
  -n, --non-interactive  Skip user input prompts
  --role <name>       Agent role (explorer, auditor, refactorer, investigator)
  --audit             Shorthand for --role auditor
  --refactor          Shorthand for --role refactorer --plan
  --plan              Enable planning mode
  --validate <cmd>    Run validation command after edits (e.g., "cargo check")
  --auto-validate     Auto-detect validation command (cargo check, tsc, etc.)
  --shadow            Edit in shadow worktree, validate before applying (enables --auto-validate)
  --auto-approve [LEVEL]  Auto-approve edits up to risk level (low/medium/high, default: low)
  --commit            Auto-commit changes after successful validation
  --retry-on-failure [N]  Retry up to N times on validation failure (default: 1)
  --diff [base]       Focus on git diff (auto-detects main/master if base omitted)
  --auto              Auto-dispatch based on task analysis
  --roles             List available roles and descriptions
  -h, --help          Show this help message

Examples:
  moss @agent "add error handling to parse_config"
  moss @agent --refactor --validate "cargo check" "extract helper function"
  moss @agent --refactor --shadow "rename foo to bar safely"
  moss @agent --audit "review security of auth module"
  moss @agent --resume abc123
]])
end

-- CLI argument parsing (delegated to agent.parser module)
M.parse_args = parser.parse_args

-- When run as script (moss @agent), execute directly
-- When required as module, return M
if args and #args >= 0 then
    local opts = M.parse_args(args)

    -- Handle --list-sessions
    if opts.list_sessions then
        local sessions = M.list_sessions()
        if #sessions == 0 then
            print("No saved sessions found.")
        else
            print("Available sessions (checkpoints):")
            for _, id in ipairs(sessions) do
                local state = M.load_checkpoint(id)
                if state then
                    local task_preview = (state.task or ""):sub(1, 50)
                    if #(state.task or "") > 50 then task_preview = task_preview .. "..." end
                    print(string.format("  %s  turn %d  %s", id, state.turn or 0, task_preview))
                else
                    print(string.format("  %s  (failed to load)", id))
                end
            end
        end
        os.exit(0)
    end

    -- Handle --list-logs
    if opts.list_logs then
        local logs = M.list_logs()
        if #logs == 0 then
            print("No session logs found.")
        else
            print("Available session logs:")
            for _, id in ipairs(logs) do
                local log_path = _moss_root .. "/.moss/agent/logs/" .. id .. ".jsonl"
                local handle = io.popen("wc -l < " .. log_path .. " 2>/dev/null")
                local line_count = handle and handle:read("*n") or 0
                if handle then handle:close() end
                print(string.format("  %s  (%d events)", id, line_count))
            end
            print("\nView with: cat .moss/agent/logs/<session-id>.jsonl | jq")
        end
        os.exit(0)
    end

    -- Handle --roles
    if opts.list_roles then
        print("Available roles:")
        print("  investigator  (default) Answer questions about the codebase")
        print("  auditor       Find issues: security, quality, patterns")
        print("  refactorer    Make code changes with validation")
        print("")
        print("Usage:")
        print("  moss @agent 'how does X work?'")
        print("  moss @agent --audit 'find unwrap on user input'")
        print("  moss @agent --refactor 'rename foo to bar'")
        print("  moss @agent --refactor --shadow 'rename foo to bar'  # safe editing via shadow worktree")
        print("  moss @agent --auto 'task'  # LLM picks the role")
        os.exit(0)
    end

    local result = M.run_state_machine(opts)
    if not result.success then
        os.exit(1)
    end
else
    return M
end
