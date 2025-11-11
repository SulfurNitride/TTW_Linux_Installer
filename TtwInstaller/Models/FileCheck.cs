namespace TtwInstaller.Models;

/// <summary>
/// Represents a file validation check from the manifest
/// </summary>
public class FileCheck
{
    /// <summary>
    /// Check type: 0=FileExists, 1=FreeSize, 2=NoProgramFiles
    /// </summary>
    public int Type { get; set; }

    /// <summary>
    /// If true, inverts the check logic (e.g., file should NOT exist)
    /// </summary>
    public bool Inverted { get; set; }

    /// <summary>
    /// Location index to check
    /// </summary>
    public int Loc { get; set; }

    /// <summary>
    /// File path to check
    /// </summary>
    public string File { get; set; } = string.Empty;

    /// <summary>
    /// Expected checksums (MD5 or SHA1, newline-separated)
    /// Auto-detected: MD5=32 chars, SHA1=40 chars
    /// Original TTW installer uses MD5
    /// </summary>
    public string? Checksums { get; set; }

    /// <summary>
    /// Custom error message to display if check fails
    /// </summary>
    public string CustomMessage { get; set; } = string.Empty;

    /// <summary>
    /// Free space required (for Type=1)
    /// </summary>
    public long FreeSize { get; set; }

    /// <summary>
    /// Parse checksums into individual SHA1 hashes
    /// </summary>
    public List<string> GetChecksumList()
    {
        if (string.IsNullOrEmpty(Checksums))
            return new List<string>();

        return Checksums
            .Split(new[] { '\r', '\n' }, StringSplitOptions.RemoveEmptyEntries)
            .Select(s => s.Trim().ToUpperInvariant())
            .Where(s => !string.IsNullOrEmpty(s))
            .ToList();
    }
}

/// <summary>
/// Check types
/// </summary>
public enum FileCheckType
{
    FileExists = 0,
    FreeSize = 1,
    NoProgramFiles = 2
}
