namespace App.Store;

/// <summary>Reads rows by id.</summary>
public interface IRepository
{
    Row? Find(int id);
}

public record Row(int Id, string Name);

public static class Limits
{
    public const int MaxId = 1000;
}

public abstract class BaseRepository
{
    protected void Check(int id)
    {
        if (id > Limits.MaxId) throw new ArgumentException("id");
    }
}
