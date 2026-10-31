open Repo

let open_store () =
  let r = add empty 1 in
  Repo.find r 0 |> string_of_int |> print_endline

module Cache = struct
  let get r = find r 0
end

let cached r = Cache.get r
