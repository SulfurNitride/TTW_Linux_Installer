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

        // Check lz4 (system only - not bundled)
        if (!CheckSystemBinary("lz4", BundledBinaryManager.GetLz4Path, BundledBinaryManager.IsLz4Available, log))
        {
            missing.Add("lz4");
        }

        if (missing.Count > 0)
        {
            log("");
            log("❌ Dependency check failed! Required dependencies are missing.");
            log("");

            // Categorize missing dependencies
            var bundledMissing = missing.Where(d => d != "lz4").ToList();
            var systemMissing = missing.Where(d => d == "lz4").ToList();

            if (bundledMissing.Count > 0)
            {
                log("   Bundled binaries missing (reinstall required):");
                foreach (var dep in bundledMissing)
                    log($"     - {dep}");
            }

            if (systemMissing.Count > 0)
            {
                log("   System dependencies missing (install via package manager):");
                foreach (var dep in systemMissing)
                    log($"     - {dep}");
            }

            log("");
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

    private static bool CheckSystemBinary(string name, Func<string> getPathFunc, Func<bool> isAvailableFunc, Action<string> log)
    {
        try
        {
            if (!isAvailableFunc())
            {
                log($"  ❌ {name}: Not found in system PATH");
                log($"     Install with: sudo pacman -S {name}");
                return false;
            }

            var path = getPathFunc();
            log($"  ✅ {name}: Found at {path}");
            return true;
        }
        catch (Exception ex)
        {
            log($"  ❌ {name}: MISSING - {ex.Message}");
            return false;
        }
    }

}
