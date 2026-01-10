-- Agent risk assessment and validation detection
-- Usage: local risk = require("agent.risk")

local M = {}

-- Risk levels for edits
M.RISK = {
    LOW = "low",       -- Comments, docs, minor tweaks
    MEDIUM = "medium", -- Function body changes, new functions
    HIGH = "high",     -- Deletions, public API, config, entry points
}

-- Risk level ordering for comparisons
local RISK_ORDER = { low = 1, medium = 2, high = 3 }

-- Assess risk level of an edit operation
-- edit_info: { action = "delete"|"replace"|"insert", target = "path/Symbol", content = "..." }
function M.assess_risk(edit_info)
    local action = edit_info.action or "replace"
    local target = edit_info.target or ""
    local content = edit_info.content or ""

    -- HIGH: Delete is always high risk
    if action == "delete" then
        return M.RISK.HIGH, "Deleting code is high risk"
    end

    -- HIGH: Config files
    local config_patterns = { "%.toml$", "%.json$", "%.yaml$", "%.yml$", "%.env", "Dockerfile", "Makefile" }
    for _, pat in ipairs(config_patterns) do
        if target:match(pat) then
            return M.RISK.HIGH, "Modifying config file"
        end
    end

    -- HIGH: Entry points and public API
    local high_risk_symbols = { "main", "run", "execute", "start", "init" }
    for _, sym in ipairs(high_risk_symbols) do
        if target:lower():match("/" .. sym .. "$") then
            return M.RISK.HIGH, "Modifying entry point or public API"
        end
    end

    -- HIGH: Changes that add pub/public visibility
    if content:match("^%s*pub%s") or content:match("^%s*public%s") or content:match("^%s*export%s") then
        return M.RISK.HIGH, "Changing public API"
    end

    -- LOW: Comments and documentation
    if content:match("^%s*//") or content:match("^%s*#") or content:match("^%s*%-%-") or
       content:match("^%s*/%*") or content:match("^%s*'''") or content:match('^%s*"""') then
        return M.RISK.LOW, "Adding comment or documentation"
    end

    -- LOW: Pure insertions (adding new code, not modifying)
    if action == "insert" then
        return M.RISK.LOW, "Inserting new code"
    end

    -- MEDIUM: Everything else (replace operations on regular code)
    return M.RISK.MEDIUM, "Modifying function or code"
end

-- Check if edit should be auto-approved based on risk threshold
-- auto_approve_level: "low", "medium", "high", or nil (no auto-approve)
-- Returns: true if should auto-approve, false if needs user confirmation
function M.should_auto_approve(edit_risk, auto_approve_level)
    if not auto_approve_level then
        return false  -- No auto-approve, always ask
    end
    local risk_value = RISK_ORDER[edit_risk] or 2
    local threshold = RISK_ORDER[auto_approve_level] or 1
    return risk_value <= threshold
end

-- Auto-detect validation command based on project files
-- Uses _moss_root global for project root
function M.detect_validator()
    -- Check for project markers in priority order
    local checks = {
        -- Rust: cargo check is fast type-checking
        { file = "Cargo.toml", cmd = "cargo check", desc = "Rust" },
        -- TypeScript: type-check without emitting
        { file = "tsconfig.json", cmd = "tsc --noEmit", desc = "TypeScript" },
        -- Go: fast compilation check
        { file = "go.mod", cmd = "go build ./...", desc = "Go" },
        -- Python: mypy if installed, else basic syntax check
        { file = "pyproject.toml", cmd = "python -m py_compile", desc = "Python" },
        -- Node.js: if package.json has typecheck script
        { file = "package.json", cmd = nil, desc = "JavaScript/Node.js" },
    }

    for _, check in ipairs(checks) do
        local f = io.open(_moss_root .. "/" .. check.file, "r")
        if f then
            f:close()
            -- Special case: check for typecheck script in package.json
            if check.file == "package.json" then
                local pf = io.open(_moss_root .. "/package.json", "r")
                if pf then
                    local content = pf:read("*a")
                    pf:close()
                    if content:match('"typecheck"') or content:match('"type%-check"') then
                        return "npm run typecheck", "Node.js (typecheck)"
                    elseif content:match('"tsc"') then
                        return "npm run tsc", "Node.js (tsc)"
                    elseif content:match('"build"') then
                        return "npm run build", "Node.js (build)"
                    end
                end
                -- No validation script found, skip
            elseif check.cmd then
                return check.cmd, check.desc
            end
        end
    end
    return nil, nil  -- No validator detected
end

return M
