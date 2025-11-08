using System.Security.Cryptography;
using System.Text;

namespace TtwInstaller.Utils;

/// <summary>
/// Utility for calculating file checksums
/// </summary>
public static class ChecksumHelper
{
    /// <summary>
    /// Calculate SHA1 checksum of a file
    /// </summary>
    public static string CalculateSHA1(string filePath)
    {
        using var sha1 = SHA1.Create();
        using var stream = System.IO.File.OpenRead(filePath);

        byte[] hash = sha1.ComputeHash(stream);

        // Convert to hex string
        var sb = new StringBuilder(hash.Length * 2);
        foreach (byte b in hash)
        {
            sb.Append(b.ToString("X2"));
        }

        return sb.ToString();
    }

    /// <summary>
    /// Calculate SHA1 checksum of byte array
    /// </summary>
    public static string CalculateSHA1(byte[] data)
    {
        using var sha1 = SHA1.Create();
        byte[] hash = sha1.ComputeHash(data);

        var sb = new StringBuilder(hash.Length * 2);
        foreach (byte b in hash)
        {
            sb.Append(b.ToString("X2"));
        }

        return sb.ToString();
    }

    /// <summary>
    /// Verify file matches one of the expected checksums
    /// </summary>
    public static bool VerifyChecksum(string filePath, List<string> expectedChecksums)
    {
        if (!System.IO.File.Exists(filePath))
            return false;

        if (expectedChecksums.Count == 0)
            return true; // No checksums to verify

        string actualChecksum = CalculateSHA1(filePath);

        return expectedChecksums.Any(expected =>
            string.Equals(expected, actualChecksum, StringComparison.OrdinalIgnoreCase));
    }
}
