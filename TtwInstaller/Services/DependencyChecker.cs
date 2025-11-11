using System.Diagnostics;
using System.Runtime.InteropServices;

namespace TtwInstaller.Services;

/// <summary>
/// Checks for required system dependencies
/// </summary>
public static class DependencyChecker
{
    public static (bool Success, List<string> MissingDependencies) CheckDependencies()
    {
        var missing = new List<string>();

        Console.WriteLine("\n=== Pre-Installation Checks ===\n");

        // xdelta3 is bundled - verify it's available
        Console.Write("  Checking xdelta3 (bundled)... ");
        try
        {
            var xdeltaPath = BundledBinaryManager.GetXdelta3Path();
            Console.WriteLine($"✅ Found at {xdeltaPath}");
        }
        catch (Exception ex)
        {
            Console.WriteLine($"❌ MISSING");
            Console.WriteLine($"     Error: {ex.Message}");
            missing.Add("xdelta3 (bundled binary missing - reinstall required)");
        }

        // Check ffmpeg (bundled or system)
        Console.Write("  Checking ffmpeg (bundled or system)... ");
        if (BundledBinaryManager.IsFfmpegAvailable())
        {
            var ffmpegPath = BundledBinaryManager.GetFfmpegPath();
            var isBundled = Path.GetDirectoryName(ffmpegPath)?.Contains(AppContext.BaseDirectory) ?? false;
            var source = isBundled ? "bundled" : "system";
            Console.WriteLine($"✅ Found ({source})");
        }
        else
        {
            missing.Add("ffmpeg");
            Console.WriteLine("❌ NOT FOUND");
        }

        Console.WriteLine();

        if (missing.Count > 0)
        {
            Console.WriteLine("❌ Pre-installation checks failed!\n");
            PrintInstallInstructions(missing);
            return (false, missing);
        }

        Console.WriteLine("✅ All pre-installation checks passed\n");
        return (true, missing);
    }

    private static bool IsCommandAvailable(string command)
    {
        try
        {
            var startInfo = new ProcessStartInfo
            {
                FileName = "which",
                Arguments = command,
                RedirectStandardOutput = true,
                RedirectStandardError = true,
                UseShellExecute = false,
                CreateNoWindow = true
            };

            using var process = Process.Start(startInfo);
            if (process == null) return false;

            process.WaitForExit(1000);
            return process.ExitCode == 0;
        }
        catch
        {
            return false;
        }
    }

    private static void PrintInstallInstructions(List<string> missing)
    {
        Console.WriteLine("Please install the missing dependencies:\n");

        // Detect distro
        string distro = DetectLinuxDistro();

        Console.WriteLine($"Detected: {distro}\n");

        switch (distro.ToLower())
        {
            case "ubuntu":
            case "debian":
            case "mint":
            case "pop":
                Console.WriteLine($"  sudo apt install {string.Join(" ", missing)}");
                break;

            case "fedora":
            case "rhel":
            case "centos":
                Console.WriteLine($"  sudo dnf install {string.Join(" ", missing)}");
                break;

            case "arch":
            case "manjaro":
            case "endeavouros":
                Console.WriteLine($"  sudo pacman -S {string.Join(" ", missing)}");
                break;

            case "opensuse":
                Console.WriteLine($"  sudo zypper install {string.Join(" ", missing)}");
                break;

            case "bazzite":
            case "silverblue":
            case "kinoite":
                Console.WriteLine("  Immutable OS detected. Install via Flatpak or layer packages:");
                Console.WriteLine($"  rpm-ostree install {string.Join(" ", missing)}");
                Console.WriteLine("  (Requires reboot)");
                break;

            default:
                Console.WriteLine($"  Install using your package manager: {string.Join(", ", missing)}");
                break;
        }

        Console.WriteLine();
    }

    private static string DetectLinuxDistro()
    {
        try
        {
            if (File.Exists("/etc/os-release"))
            {
                var lines = File.ReadAllLines("/etc/os-release");
                foreach (var line in lines)
                {
                    if (line.StartsWith("ID="))
                    {
                        return line.Substring(3).Trim('"').ToLower();
                    }
                }
            }
        }
        catch { }

        return "Linux";
    }
}
