defmodule Store.RepoTest do
  use ExUnit.Case
  alias Store.Repo

  test "get missing" do
    assert Repo.get(%{}, 1) == {:error, :missing}
  end
end
