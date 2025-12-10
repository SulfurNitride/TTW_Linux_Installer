using System.Runtime.InteropServices;

namespace TtwInstaller.Services;

/// <summary>
/// Manages bundled native binaries (xdelta3, ffmpeg, lz4)
///
/// Bundled binaries (shipped with application):
/// - xdelta3: v3.1.0 (https://github.com/jmacd/xdelta)
///   SHA1 (linux-x64): b64031ee8450f148a52bc10ff82e46bdee245ea2
/// - ffmpeg: Bundled with application
/// - lz4: Bundled with application
///
/// See BUNDLED_BINARIES.md for full details on versions and sources.
/// </summary>
public static class BundledBinaryManager
{
    // Bundled binary versions - update when binaries are updated
    public const string XDELTA3_VERSION = "3.1.0";
    public const string XDELTA3_SOURCE = "https://github.com/jmacd/xdelta";

    private static string? _xdelta3Path;
    private static string? _ffmpegPath;
    private static string? _lz4Path;
    private static readonly object _lock = new();

    /// <summary>
    /// Get path to bundled xdelta3 binary
    /// </summary>
    public static string GetXdelta3Path()
    {
        lock (_lock)
        {
            if (_xdelta3Path != null)
                return _xdelta3Path;

            // 1. Try flattened path (release/single-file)
            var appDir = AppContext.BaseDirectory;
            var flattenedPath = Path.Combine(appDir, "xdelta3");
            if (File.Exists(flattenedPath))
            {
                MakeExecutable(flattenedPath);
                _xdelta3Path = flattenedPath;
                return _xdelta3Path;
            }

            // 2. Try structured path (dev/cli)
            var structuredPath = GetBundledBinaryPath("xdelta3");
            if (structuredPath != null)
            {
                MakeExecutable(structuredPath);
                _xdelta3Path = structuredPath;
                return _xdelta3Path;
            }

            throw new FileNotFoundException($"Bundled xdelta3 not found at expected path: {flattenedPath}");
        }
    }

    /// <summary>
    /// Get path to ffmpeg binary (bundled or system)
    /// Tries bundled version first, falls back to system PATH
    /// </summary>
    public static string GetFfmpegPath()
    {
        lock (_lock)
        {
            if (_ffmpegPath != null)
                return _ffmpegPath;

            // 1. Try flattened path (release/single-file)
            var appDir = AppContext.BaseDirectory;
            var flattenedPath = Path.Combine(appDir, "ffmpeg");
            if (File.Exists(flattenedPath))
            {
                MakeExecutable(flattenedPath);
                _ffmpegPath = flattenedPath;
                return _ffmpegPath;
            }

            // 2. Try structured path (dev/cli)
            var structuredPath = GetBundledBinaryPath("ffmpeg");
            if (structuredPath != null)
            {
                MakeExecutable(structuredPath);
                _ffmpegPath = structuredPath;
                return _ffmpegPath;
            }

            // 3. Fall back to system ffmpeg
            var systemPath = FindSystemBinary("ffmpeg");
            if (systemPath != null)
            {
                _ffmpegPath = systemPath;
                return _ffmpegPath;
            }

            // Return "ffmpeg" as last resort
            return "ffmpeg";
        }
    }

    /// <summary>
    /// Get path to lz4 binary (bundled or system)
    /// </summary>
    public static string GetLz4Path()
    {
        lock (_lock)
        {
            if (_lz4Path != null)
                return _lz4Path;

            // 1. Try flattened path (release/single-file)
            var appDir = AppContext.BaseDirectory;
            var flattenedPath = Path.Combine(appDir, "lz4");
            if (File.Exists(flattenedPath))
            {
                MakeExecutable(flattenedPath);
                _lz4Path = flattenedPath;
                return _lz4Path;
            }

            // 2. Try structured path (dev/cli)
            var structuredPath = GetBundledBinaryPath("lz4");
            if (structuredPath != null)
            {
                MakeExecutable(structuredPath);
                _lz4Path = structuredPath;
                return _lz4Path;
            }

            // 3. Fall back to system lz4
            var systemPath = FindSystemBinary("lz4");
            if (systemPath != null)
            {
                _lz4Path = systemPath;
                return _lz4Path;
            }

            // Return "lz4" as last resort
            return "lz4";
        }
    }

    /// <summary>
    /// Check if xdelta3 is available (bundled or system)
    /// </summary>
    public static bool IsXdelta3Available()
    {
        try
        {
            GetXdelta3Path();
            return true;
        }
        catch
        {
            return false;
        }
    }

    /// <summary>
    /// Check if ffmpeg is available (bundled or system)
    /// </summary>
    public static bool IsFfmpegAvailable()
    {
        var path = GetFfmpegPath();
        if (path == "ffmpeg")
            return FindSystemBinary("ffmpeg") != null;
        return File.Exists(path);
    }

    /// <summary>
    /// Check if lz4 is available (bundled or system)
    /// </summary>
    public static bool IsLz4Available()
    {
        var path = GetLz4Path();
        if (path == "lz4")
            return FindSystemBinary("lz4") != null;
        return File.Exists(path);
    }

    /// <summary>
    /// Get path to bundled binary for current platform (Linux only)
    /// </summary>
    private static string? GetBundledBinaryPath(string binaryName)
    {
        try
        {
            var appDir = AppContext.BaseDirectory;
            string platformDir = RuntimeInformation.ProcessArchitecture == Architecture.X64
                ? "linux-x64"
                : "linux-arm64";

            var bundledPath = Path.Combine(appDir, "BundledBinaries", platformDir, binaryName);
            return File.Exists(bundledPath) ? bundledPath : null;
        }
        catch
        {
            return null;
        }
    }

    /// <summary>
    /// Find a binary in system PATH
    /// </summary>
    private static string? FindSystemBinary(string binaryName)
    {
        try
        {
            var startInfo = new System.Diagnostics.ProcessStartInfo
            {
                FileName = "which",
                Arguments = binaryName,
                RedirectStandardOutput = true,
                RedirectStandardError = true,
                UseShellExecute = false,
                CreateNoWindow = true
            };

            using var process = System.Diagnostics.Process.Start(startInfo);
            if (process == null) return null;

            var output = process.StandardOutput.ReadToEnd().Trim();
            process.WaitForExit(1000);

            if (process.ExitCode == 0 && !string.IsNullOrWhiteSpace(output))
            {
                return output.Split('\n')[0].Trim();
            }
        }
        catch { }

        return null;
    }

    /// <summary>
    /// Ensure a file is executable (chmod +x)
    /// </summary>
    private static void MakeExecutable(string path)
    {
        try
        {
            var chmod = new System.Diagnostics.ProcessStartInfo
            {
                FileName = "chmod",
                Arguments = $"+x \"{path}\"",
                RedirectStandardOutput = true,
                RedirectStandardError = true,
                UseShellExecute = false,
                CreateNoWindow = true
            };
            using var process = System.Diagnostics.Process.Start(chmod);
            process?.WaitForExit(1000);
        }
        catch { }
    }
}