local repo = require("app.repo")
local put = repo.put
local insert = table.insert

local App = {}

function App:find(id)
  put(self.rows, id, nil)
  return repo.get(self.rows, id)
end

function App.new()
  local url = os.getenv("APP_URL")
  local app = setmetatable({ rows = {}, url = url }, { __index = App })
  repo.put(app.rows, 1, "first")
  insert(app.rows, "third")
  return app
end

return App
