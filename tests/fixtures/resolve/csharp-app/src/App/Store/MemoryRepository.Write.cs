namespace App.Store;

public partial class MemoryRepository
{
    public void Put(Row row) => rows[row.Id] = row;

    public void Put(int id, string name) => Put(new Row(id, name));
}

public static class RowExtensions
{
    public static string Label(this Row row) => row.Name.ToUpperInvariant();
}
