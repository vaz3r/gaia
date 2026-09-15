namespace Gaia.Sync;

public record KeysetCursor(DateTime? Timestamp, string Hash)
{
    public static readonly KeysetCursor Initial = new(null, string.Empty);

    public bool IsInitial => !Timestamp.HasValue && string.IsNullOrEmpty(Hash);
}
