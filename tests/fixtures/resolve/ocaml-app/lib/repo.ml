type t = { rows : int list }

let empty = { rows = [] }

let find r id = List.nth r.rows id

let add r x = { rows = x :: r.rows }
