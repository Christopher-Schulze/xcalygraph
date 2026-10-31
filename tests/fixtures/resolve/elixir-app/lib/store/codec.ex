defmodule Store.Codec do
  # `import` makes the module's functions callable without qualifier; a local function named
  # like a builtin (`derive`) is still the local one.
  import Store.Repo

  def load(id) do
    id |> get() |> derive()
  end

  defp derive(row), do: row
end
