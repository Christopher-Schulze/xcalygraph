-module(store_server).
-behaviour(gen_server).
-export([start_link/0, init/1, handle_call/3]).

start_link() ->
    gen_server:start_link({local, ?MODULE}, ?MODULE, [], []).

init([]) ->
    Url = os:getenv("STORE_URL"),
    {ok, #{url => Url, rows => #{}}}.

handle_call({get, Id}, _From, State) ->
    {reply, store_repo:get(maps:get(rows, State), Id), State}.
