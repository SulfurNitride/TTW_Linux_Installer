using System.Diagnostics;
using System.Runtime.InteropServices;

namespace TtwInstaller.Services;

/// <summary>
/// Wrapper for bsarch.exe command-line tool (Windows only)
/// Used as alternative to libbsa on Windows for better compatibility
/// </summary>
public static class BsarchWrapper
{
    private static string? _bsarchPath;
    private static readonly object _lock = new();

    /// <summary>
    /// Get path to bundled bsarch.exe (Windows only)
    /// </summary>
    private static string GetBsarchPath()
    {
        lock (_lock)
        {
            if (_bsarchPath != null)
                return _bsarchPath;

            if (!RuntimeInformation.IsOSPlatform(OSPlatform.Windows))
            {
                throw new PlatformNotSupportedException("bsarch.exe is only available on Windows");
            }

            var appDir = AppContext.BaseDirectory;
            var bsarchExe = Path.Combine(appDir, "bsarch.exe");

            if (!File.Exists(bsarchExe))
            {
                throw new FileNotFoundException($"bsarch.exe not found at: {bsarchExe}");
            }

            _bsarchPath = bsarchExe;
            return _bsarchPath;
        }
    }

    /// <summary>
    /// Extract a single file from BSA archive
    /// </summary>
    public static byte[]? ExtractFile(string bsaPath, string filePath)
    {
        // Use temp directory for extraction
        var tempDir = Path.Combine(Path.GetTempPath(), $"bsarch_{Guid.NewGuid():N}");

        try
        {
            Directory.CreateDirectory(tempDir);

            // bsarch unpack command extracts entire BSA - we'll extract all then read the file we need
            // This is inefficient but bsarch doesn't support single-file extraction
            var args = $"unpack \"{bsaPath}\" \"{tempDir}\"";

            var result = RunBsarch(args, timeout: 300000); // 5 minute timeout for large BSAs

            if (result.ExitCode != 0)
            {
                Console.WriteLine($"Warning: bsarch extraction failed for {bsaPath}: {result.Error}");
                return null;
            }

            // Find the extracted file (case-insensitive on Windows)
            var extractedFile = Path.Combine(tempDir, filePath);

            if (!File.Exists(extractedFile))
            {
                // Try case-insensitive search
                extractedFile = FindFileCaseInsensitive(tempDir, filePath);
            }

            if (extractedFile != null && File.Exists(extractedFile))
            {
                return File.ReadAllBytes(extractedFile);
            }

            return null;
        }
        finally
        {
            // Cleanup temp directory
            try
            {
                if (Directory.Exists(tempDir))
                {
                    Directory.Delete(tempDir, recursive: true);
                }
            }
            catch { /* Ignore cleanup errors */ }
        }
    }

    /// <summary>
    /// Check if a file exists in BSA archive
    /// Note: This requires unpacking the BSA to check, which is slow
    /// Consider caching BSA contents if calling this frequently
    /// </summary>
    public static bool FileExists(string bsaPath, string filePath)
    {
        // For performance, we could dump the file list instead of extracting
        var args = $"\"{bsaPath}\" -dump";
        var result = RunBsarch(args, timeout: 60000);

        if (result.ExitCode != 0)
            return false;

        // Check if the file path appears in the dump output
        var normalizedPath = filePath.Replace('/', '\\').ToLowerInvariant();
        return result.Output.Contains(normalizedPath, StringComparison.OrdinalIgnoreCase);
    }

    /// <summary>
    /// Pack a directory into a BSA archive
    /// </summary>
    public static bool PackArchive(string sourceDir, string bsaPath, string gameType = "fnv",
        int? archiveFlags = null, int? fileFlags = null, bool compress = true, bool multiThreaded = true)
    {
        var args = $"pack \"{sourceDir}\" \"{bsaPath}\" -{gameType}";

        if (archiveFlags.HasValue)
            args += $" -af:0x{archiveFlags.Value:X}";

        if (fileFlags.HasValue)
            args += $" -ff:0x{fileFlags.Value:X}";

        if (compress)
            args += " -z";

        if (multiThreaded)
            args += " -mt";

        var result = RunBsarch(args, timeout: 600000); // 10 minute timeout for packing

        if (result.ExitCode != 0)
        {
            Console.WriteLine($"Error: bsarch pack failed: {result.Error}");
            return false;
        }

        return true;
    }

    /// <summary>
    /// Run bsarch.exe with arguments
    /// </summary>
    private static (int ExitCode, string Output, string Error) RunBsarch(string arguments, int timeout = 120000)
    {
        var bsarchExe = GetBsarchPath();

        var startInfo = new ProcessStartInfo
        {
            FileName = bsarchExe,
            Arguments = arguments,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            UseShellExecute = false,
            CreateNoWindow = true,
            WorkingDirectory = Path.GetDirectoryName(bsarchExe) ?? AppContext.BaseDirectory
        };

        using var process = Process.Start(startInfo);
        if (process == null)
        {
            return (-1, "", "Failed to start bsarch.exe");
        }

        var outputTask = process.StandardOutput.ReadToEndAsync();
        var errorTask = process.StandardError.ReadToEndAsync();

        if (!process.WaitForExit(timeout))
        {
            process.Kill();
            return (-1, "", "bsarch.exe timed out");
        }

        var output = outputTask.Result;
        var error = errorTask.Result;

        return (process.ExitCode, output, error);
    }

    /// <summary>
    /// Find a file case-insensitively in a directory tree
    /// </summary>
    private static string? FindFileCaseInsensitive(string baseDir, string relativePath)
    {
        try
        {
            var parts = relativePath.Split(new[] { '/', '\\' }, StringSplitOptions.RemoveEmptyEntries);
            var currentDir = baseDir;

            for (int i = 0; i < parts.Length - 1; i++)
            {
                var matchingDir = Directory.GetDirectories(currentDir)
                    .FirstOrDefault(d => Path.GetFileName(d).Equals(parts[i], StringComparison.OrdinalIgnoreCase));

                if (matchingDir == null)
                    return null;

                currentDir = matchingDir;
            }

            // Find the file in the final directory
            var fileName = parts[^1];
            var matchingFile = Directory.GetFiles(currentDir)
                .FirstOrDefault(f => Path.GetFileName(f).Equals(fileName, StringComparison.OrdinalIgnoreCase));

            return matchingFile;
        }
        catch
        {
            return null;
        }
    }
}
