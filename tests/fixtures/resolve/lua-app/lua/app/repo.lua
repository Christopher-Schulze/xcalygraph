local M = {}

local function check(row)
  if row == nil then
    return nil, "missing"
  end
  return row
end

function M.get(rows, id)
  return check(rows[id])
end

function M.put(rows, id, row)
  rows[id] = row
end

return M
