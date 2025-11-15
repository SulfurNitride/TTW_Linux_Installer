using System.Diagnostics;
using System.Runtime.InteropServices;

namespace TtwInstaller.Services;

/// <summary>
/// Checks for required system dependencies
/// </summary>
public static class DependencyChecker
{
    public static (bool Success, List<string> MissingDependencies) CheckDependencies(Action<string>? logger = null)
    {
        var missing = new List<string>();
        var log = logger ?? Console.WriteLine;

        // Check xdelta3 (bundled only)
        if (!CheckBundledBinary("xdelta3", BundledBinaryManager.GetXdelta3Path, log))
        {
            missing.Add("xdelta3");
        }

        // Check ffmpeg (bundled only)
        if (!CheckBundledBinary("ffmpeg", BundledBinaryManager.GetFfmpegPath, log))
        {
            missing.Add("ffmpeg");
        }

        // Check lz4 (bundled only)
        if (!CheckBundledBinary("lz4", BundledBinaryManager.GetLz4Path, log))
        {
            missing.Add("lz4");
        }

        if (missing.Count > 0)
        {
            log("");
            log("❌ Dependency check failed! Required bundled binaries are missing.");
            log("   Please reinstall the application or verify the installation is complete.");
            log("");
            log($"   Missing: {string.Join(", ", missing)}");
            return (false, missing);
        }

        log("✅ All dependency checks passed");
        return (true, missing);
    }

    private static bool CheckBundledBinary(string name, Func<string> getPathFunc, Action<string> log)
    {
        try
        {
            var path = getPathFunc();

            // Verify it's actually in the app directory (bundled)
            var appDir = AppContext.BaseDirectory;
            if (!path.StartsWith(appDir))
            {
                log($"  ❌ {name}: Found at system path instead of bundled - not supported");
                return false;
            }

            // Verify file exists
            if (!File.Exists(path))
            {
                log($"  ❌ {name}: Not found at expected path: {path}");
                return false;
            }

            log($"  ✅ {name}: Found (bundled)");
            return true;
        }
        catch (Exception ex)
        {
            log($"  ❌ {name}: MISSING - {ex.Message}");
            return false;
        }
    }

}
