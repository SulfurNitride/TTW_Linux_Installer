using System.Security.Cryptography;
using System.Text;

namespace TtwInstaller.Utils;

/// <summary>
/// Utility for calculating file checksums
/// </summary>
public static class ChecksumHelper
{
    /// <summary>
    /// Calculate MD5 checksum of a file (used by original TTW installer)
    /// </summary>
    public static string CalculateMD5(string filePath)
    {
        using var md5 = MD5.Create();
        using var stream = System.IO.File.OpenRead(filePath);

        byte[] hash = md5.ComputeHash(stream);

        // Convert to hex string (uppercase to match original installer)
        var sb = new StringBuilder(hash.Length * 2);
        foreach (byte b in hash)
        {
            sb.Append(b.ToString("X2"));
        }

        return sb.ToString();
    }

    /// <summary>
    /// Calculate MD5 checksum of byte array
    /// </summary>
    public static string CalculateMD5(byte[] data)
    {
        using var md5 = MD5.Create();
        byte[] hash = md5.ComputeHash(data);

        var sb = new StringBuilder(hash.Length * 2);
        foreach (byte b in hash)
        {
            sb.Append(b.ToString("X2"));
        }

        return sb.ToString();
    }

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
    /// Tries MD5, SHA1, and SHA256 - matches if ANY algorithm matches
    /// </summary>
    public static bool VerifyChecksum(string filePath, List<string> expectedChecksums)
    {
        if (!System.IO.File.Exists(filePath))
            return false;

        if (expectedChecksums.Count == 0)
            return true; // No checksums to verify

        // Calculate all hash types upfront
        string md5Hash = CalculateMD5(filePath);
        string sha1Hash = CalculateSHA1(filePath);

        // Check if ANY expected checksum matches ANY hash type
        foreach (var expected in expectedChecksums)
        {
            var trimmed = expected.Trim();

            // Try MD5 (32 chars)
            if (trimmed.Length == 32 && string.Equals(trimmed, md5Hash, StringComparison.OrdinalIgnoreCase))
                return true;

            // Try SHA1 (40 chars)
            if (trimmed.Length == 40 && string.Equals(trimmed, sha1Hash, StringComparison.OrdinalIgnoreCase))
                return true;
        }

        return false;
    }

    /// <summary>
    /// Calculate checksum using appropriate algorithm based on expected format
    /// Returns the hash that matches the expected length
    /// </summary>
    public static string CalculateChecksum(string filePath, string? sampleHash = null)
    {
        if (!string.IsNullOrEmpty(sampleHash))
        {
            var trimmed = sampleHash.Trim();

            // MD5 = 32 chars
            if (trimmed.Length == 32)
                return CalculateMD5(filePath);

            // SHA1 = 40 chars
            if (trimmed.Length == 40)
                return CalculateSHA1(filePath);
        }

        // Default to MD5 (original TTW installer format)
        return CalculateMD5(filePath);
    }
}
