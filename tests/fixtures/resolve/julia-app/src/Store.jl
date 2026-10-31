module Store

include("util.jl")
using .Util

export Repo, open_store

struct Repo
    rows::Dict{Int,String}
end

function open_store(path::String)
    r = Repo(Dict{Int,String}())
    Util.check(r.rows)
    normalize(path)
    return r
end

end
