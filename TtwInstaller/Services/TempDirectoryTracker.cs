namespace TtwInstaller.Services;

/// <summary>
/// Tracks temporary directories created during installation for cleanup
/// </summary>
public static class TempDirectoryTracker
{
    private static readonly HashSet<string> _trackedDirectories = new();
    private static readonly object _lock = new();

    /// <summary>
    /// Register a temporary directory for cleanup
    /// </summary>
    public static void Register(string directory)
    {
        lock (_lock)
        {
            _trackedDirectories.Add(directory);
        }
    }

    /// <summary>
    /// Unregister a temporary directory (already cleaned up)
    /// </summary>
    public static void Unregister(string directory)
    {
        lock (_lock)
        {
            _trackedDirectories.Remove(directory);
        }
    }

    /// <summary>
    /// Clean up all tracked temporary directories
    /// </summary>
    public static void CleanupAll()
    {
        List<string> directories;
        lock (_lock)
        {
            directories = new List<string>(_trackedDirectories);
            _trackedDirectories.Clear();
        }

        foreach (var dir in directories)
        {
            try
            {
                if (Directory.Exists(dir))
                {
                    Directory.Delete(dir, true);
                }
            }
            catch
            {
                // Ignore cleanup errors
            }
        }
    }
}
