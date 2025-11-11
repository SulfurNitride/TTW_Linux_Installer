using System.Runtime.InteropServices;

namespace TtwInstaller.Services;

/// <summary>
/// Manages bundled native binaries (xdelta3, ffmpeg, etc.)
///
/// Version Tracking:
/// - xdelta3: v3.1.0 (https://github.com/jmacd/xdelta)
///   SHA1 (linux-x64): b64031ee8450f148a52bc10ff82e46bdee245ea2
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

            // Look for xdelta3 next to the executable
            var appDir = AppContext.BaseDirectory;
            var bundledPath = Path.Combine(appDir, "xdelta3");

            if (File.Exists(bundledPath))
            {
                // Make sure it's executable
                try
                {
                    var chmod = new System.Diagnostics.ProcessStartInfo
                    {
                        FileName = "chmod",
                        Arguments = $"+x \"{bundledPath}\"",
                        RedirectStandardOutput = true,
                        RedirectStandardError = true,
                        UseShellExecute = false,
                        CreateNoWindow = true
                    };
                    using var process = System.Diagnostics.Process.Start(chmod);
                    process?.WaitForExit(1000);
                }
                catch { }

                _xdelta3Path = bundledPath;
                return _xdelta3Path;
            }

            throw new FileNotFoundException($"Bundled xdelta3 not found at expected path: {bundledPath}");
        }
    }

    /// <summary>
    /// Get path to bundled binary for current platform (Linux only)
    /// </summary>
    private static string? GetBundledBinaryPath(string binaryName)
    {
        try
        {
            // Get the directory where the executable is located
            var appDir = AppContext.BaseDirectory;

            // Determine platform subdirectory
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
                // Take first line if multiple results
                return output.Split('\n')[0].Trim();
            }
        }
        catch { }

        return null;
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
    /// Get path to ffmpeg binary (bundled or system)
    /// Tries bundled version first, falls back to system PATH
    /// </summary>
    public static string GetFfmpegPath()
    {
        lock (_lock)
        {
            if (_ffmpegPath != null)
                return _ffmpegPath;

            // Try bundled version first
            var appDir = AppContext.BaseDirectory;
            var bundledPath = Path.Combine(appDir, "ffmpeg");

            if (File.Exists(bundledPath))
            {
                // Make executable
                try
                {
                    var chmod = new System.Diagnostics.ProcessStartInfo
                    {
                        FileName = "chmod",
                        Arguments = $"+x \"{bundledPath}\"",
                        RedirectStandardOutput = true,
                        RedirectStandardError = true,
                        UseShellExecute = false,
                        CreateNoWindow = true
                    };
                    using var process = System.Diagnostics.Process.Start(chmod);
                    process?.WaitForExit(1000);
                }
                catch { }

                _ffmpegPath = bundledPath;
                return _ffmpegPath;
            }

            // Fall back to system ffmpeg
            var systemPath = FindSystemBinary("ffmpeg");
            if (systemPath != null)
            {
                _ffmpegPath = systemPath;
                return _ffmpegPath;
            }

            // Return "ffmpeg" as last resort - will fail at runtime but allows dependency checking
            return "ffmpeg";
        }
    }

    /// <summary>
    /// Check if ffmpeg is available (bundled or system)
    /// </summary>
    public static bool IsFfmpegAvailable()
    {
        var path = GetFfmpegPath();

        // If it's just "ffmpeg" without a path, check if it's in PATH
        if (path == "ffmpeg")
        {
            return FindSystemBinary("ffmpeg") != null;
        }

        return File.Exists(path);
    }
}
