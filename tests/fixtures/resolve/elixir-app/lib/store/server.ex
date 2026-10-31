defmodule Store.Server do
  use GenServer
  alias Store.Repo

  def start_link(opts), do: GenServer.start_link(__MODULE__, opts, name: __MODULE__)

  def init(_opts) do
    url = System.get_env("STORE_URL")
    {:ok, %{url: url, rows: %{}}}
  end

  def handle_call({:get, id}, _from, state) do
    {:reply, Repo.get(state.rows, id), state}
  end

  def handle_cast({:put, id, row}, state) do
    {:noreply, %{state | rows: Store.Repo.put(state.rows, id, row)}}
  end
end
