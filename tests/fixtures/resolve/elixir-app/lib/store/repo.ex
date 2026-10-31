defmodule Store.Repo do
  @moduledoc "In-memory rows."

  def get(rows, id) do
    rows |> Map.get(id) |> check()
  end

  def put(rows, id, row), do: Map.put(rows, id, row)

  defp check(nil), do: {:error, :missing}
  defp check(row), do: {:ok, row}
end
