defmodule A do
  @spec run(Plug.Conn.t()) :: t
  def run(c), do: c.status
end
