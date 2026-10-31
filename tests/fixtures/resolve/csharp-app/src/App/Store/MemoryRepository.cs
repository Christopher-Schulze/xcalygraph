using System.Collections.Generic;

namespace App.Store;

public partial class MemoryRepository : BaseRepository, IRepository
{
    private readonly Dictionary<int, Row> rows = new();

    public Row? Find(int id)
    {
        Check(id);
        return rows.GetValueOrDefault(id);
    }
}
