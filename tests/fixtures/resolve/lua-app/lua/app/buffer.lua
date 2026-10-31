-- Module table: functions are defined and called as members of `buffer`.
local buffer = {}

local Reader = {}

function buffer.normalize(text)
  return text:lower()
end

function buffer.read(text)
  return buffer.normalize(text)
end

function Reader:next()
  return self:peek()
end

function Reader:peek()
  return nil
end

return buffer
