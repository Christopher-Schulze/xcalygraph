module Util

check(rows) = isempty(rows)

function normalize(path)
    return lowercase(path)
end

function first_key(rows::Dict{K,V})::K where {K,V}
    return check(rows) ? zero(K) : first(keys(rows))
end

last_key(rows::Dict{K,V}) where {K,V} = first_key(rows)

struct Box
    items::Vector{Int}
end

Base.isempty(b::Box) = length(b.items) == 0

empty_box(b::Box) = isempty(b)

end
