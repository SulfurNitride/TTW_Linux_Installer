using Avalonia;
using System;
using TtwInstaller.Models;
using TtwInstaller.Services;
using System.IO;
using System.Collections.Generic;
using TtwInstaller;

namespace TtwInstallerGui;

sealed class Program
{
    [STAThread]
    public static int Main(string[] args)
    {
        // If command-line arguments are provided, run in CLI mode
        if (args.Length > 0 && !Array.Exists(args, arg => arg == "--gui"))
        {
            return RunCliMode(args);
        }

        // Otherwise, launch GUI
        BuildAvaloniaApp().StartWithClassicDesktopLifetime(args);
        return 0;
    }

    private static int RunCliMode(string[] args)
    {
        // Use the original CLI installer logic
        var configFile = "ttw-config.json";
        var config = InstallConfig.FromFile(configFile);

        if (config == null)
        {
            config = InstallConfig.FromArgs(args);

            bool missingPaths = string.IsNullOrWhiteSpace(config.Fallout3Root) ||
                               string.IsNullOrWhiteSpace(config.FalloutNVRoot) ||
                               string.IsNullOrWhiteSpace(config.MpiPackagePath) ||
                               string.IsNullOrWhiteSpace(config.DestinationPath);

            if (missingPaths)
            {
                Console.WriteLine("❌ Missing required paths. Please provide configuration via:");
                Console.WriteLine("   1. Command-line arguments (run with --help for details)");
                Console.WriteLine("   2. Create a ttw-config.json file in the current directory");
                Console.WriteLine("   3. Run without arguments to launch GUI mode");
                Console.WriteLine();
                InstallConfig.FromArgs(new[] { "--help" });
                return 1;
            }
        }
        else
        {
            Console.WriteLine($"Loaded configuration from {configFile}\n");
            var cmdLineConfig = InstallConfig.FromArgs(args);
            config.StartInstallation = cmdLineConfig.StartInstallation;
        }

        // Check if --start flag was provided
        if (!config.StartInstallation)
        {
            Console.WriteLine("⚠️  Installation ready but not started.");
            Console.WriteLine();
            Console.WriteLine("To begin installation, run with the --start flag:");
            if (File.Exists(configFile))
            {
                Console.WriteLine("  ./TtwInstaller --start");
            }
            else
            {
                Console.WriteLine("  ./TtwInstaller --fo3 <path> --fnv <path> --mpi <path> --output <path> --start");
            }
            Console.WriteLine();
            Console.WriteLine("Or run without arguments to use GUI mode.");
            Console.WriteLine("Run with --help for more information.");
            return 0;
        }

        Console.WriteLine("🚀 Starting installation...\n");

        return TtwInstaller.Program.RunInstallation(config);
    }

    // Avalonia configuration, don't remove; also used by visual designer.
    public static AppBuilder BuildAvaloniaApp()
        => AppBuilder.Configure<App>()
            .UsePlatformDetect()
            .WithInterFont()
            .LogToTrace();
}
