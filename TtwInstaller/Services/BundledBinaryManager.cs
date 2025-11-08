using System.Runtime.InteropServices;

namespace TtwInstaller.Services;

/// <summary>
/// Manages bundled native binaries (xdelta3, ffmpeg, etc.)
/// </summary>
public static class BundledBinaryManager
{
    private static string? _xdelta3Path;
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

            // Look for xdelta3 next to the executable (same as libbsa_capi.so)
            var appDir = AppContext.BaseDirectory;
            var bundledPath = Path.Combine(appDir, "xdelta3");

            if (File.Exists(bundledPath))
            {
                // Make sure it's executable on Unix
                if (!RuntimeInformation.IsOSPlatform(OSPlatform.Windows))
                {
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
                }

                _xdelta3Path = bundledPath;
                return _xdelta3Path;
            }

            throw new FileNotFoundException($"Bundled xdelta3 not found at expected path: {bundledPath}");
        }
    }

    /// <summary>
    /// Get path to bundled binary for current platform
    /// </summary>
    private static string? GetBundledBinaryPath(string binaryName)
    {
        try
        {
            // Get the directory where the executable is located
            var appDir = AppContext.BaseDirectory;

            // Determine platform subdirectory
            string platformDir;
            if (RuntimeInformation.IsOSPlatform(OSPlatform.Windows))
            {
                platformDir = RuntimeInformation.ProcessArchitecture == Architecture.X64
                    ? "win-x64"
                    : "win-x86";
                binaryName += ".exe";
            }
            else if (RuntimeInformation.IsOSPlatform(OSPlatform.Linux))
            {
                platformDir = RuntimeInformation.ProcessArchitecture == Architecture.X64
                    ? "linux-x64"
                    : "linux-arm64";
            }
            else if (RuntimeInformation.IsOSPlatform(OSPlatform.OSX))
            {
                platformDir = RuntimeInformation.ProcessArchitecture == Architecture.Arm64
                    ? "osx-arm64"
                    : "osx-x64";
            }
            else
            {
                return null;
            }

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
            var searchCommand = RuntimeInformation.IsOSPlatform(OSPlatform.Windows) ? "where" : "which";

            var startInfo = new System.Diagnostics.ProcessStartInfo
            {
                FileName = searchCommand,
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
}
