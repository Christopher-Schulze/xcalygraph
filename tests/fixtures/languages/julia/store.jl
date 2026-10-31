module Store
using Base.Threads
import ..Util: helper
include("repo.jl")
export open_store
struct Repo <: AbstractRepo
    rows::Dict{Int,String}
end
abstract type AbstractRepo end
function open_store(path::String)
    r = Repo(Dict())
    helper(r)
    Util.check(r.rows)
    return get(ENV, "STORE_URL", "")
end
size(r::Repo) = length(r.rows)
end
x = ENV["STORE_URL"]
const LIMIT = 10
Base.show(io::IO, r::Repo) = print(io, r)
@testset "store" begin
    @test open_store("a") == ""
end
macro m(x) end
mutable struct Node; next end
y = r |> helper
