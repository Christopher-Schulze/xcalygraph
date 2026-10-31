using App.Store;
using Xunit;

namespace App.Tests;

public class MemoryRepositoryTests
{
    [Fact]
    public void FindsStoredRow()
    {
        var repository = new MemoryRepository();
        repository.Put(new Row(1, "a"));
        repository.Find(1);
    }
}
