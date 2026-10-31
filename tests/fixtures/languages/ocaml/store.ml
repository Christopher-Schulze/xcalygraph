open Util
module Repo = struct
  type t = { rows : int list }
  let find r id = List.nth r.rows id
end
let open_store path =
  let r = Repo.find { rows = [] } 0 in
  helper r;
  Sys.getenv "STORE_URL"
let limit = 10
let apply f = f |> List.map helper
external c_open : string -> int = "c_open"
let () = print_endline (Repo.find r 1)
