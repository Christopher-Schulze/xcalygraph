-module(store_repo).
-include("records.hrl").
-export([get/2, put/2]).

get(Rows, Id) ->
    check(maps:get(Id, Rows, undefined)).

put(Rows, #row{id = Id} = Row) ->
    maps:put(Id, Row, Rows).

check(undefined) -> {error, missing};
check(Row) -> {ok, Row}.
