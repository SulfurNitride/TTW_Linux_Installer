namespace TtwInstaller.Models;

/// <summary>
/// Installation configuration and paths
/// </summary>
public class InstallConfig
{
    /// <summary>
    /// Fallout 3 root directory
    /// </summary>
    public string Fallout3Root { get; set; } = string.Empty;

    /// <summary>
    /// Fallout 3 Data directory
    /// </summary>
    public string Fallout3Data => Path.Combine(Fallout3Root, "Data");

    /// <summary>
    /// Fallout New Vegas root directory
    /// </summary>
    public string FalloutNVRoot { get; set; } = string.Empty;

    private string? _falloutNVDataOverride;

    /// <summary>
    /// Fallout New Vegas Data directory
    /// Can be overridden for special cases like BSA decompression
    /// </summary>
    public string FalloutNVData
    {
        get => _falloutNVDataOverride ?? Path.Combine(FalloutNVRoot, "Data");
        set => _falloutNVDataOverride = value;
    }

    /// <summary>
    /// Oblivion root directory (TES4)
    /// </summary>
    public string OblivionRoot { get; set; } = string.Empty;

    private string? _oblivionDataOverride;

    /// <summary>
    /// Oblivion Data directory (TES4DATA)
    /// Can be overridden for special cases like BSA decompression
    /// </summary>
    public string OblivionData
    {
        get => _oblivionDataOverride ?? Path.Combine(OblivionRoot, "Data");
        set => _oblivionDataOverride = value;
    }

    /// <summary>
    /// Destination directory for TTW installation
    /// </summary>
    public string DestinationPath { get; set; } = string.Empty;

    /// <summary>
    /// Path to extracted MPI package
    /// </summary>
    public string MpiPackagePath { get; set; } = string.Empty;

    /// <summary>
    /// Validate configuration
    /// </summary>
    public void Validate()
    {
        if (string.IsNullOrWhiteSpace(Fallout3Root))
            throw new InvalidOperationException("Fallout 3 root path is required");

        if (!Directory.Exists(Fallout3Root))
            throw new DirectoryNotFoundException($"Fallout 3 directory not found: {Fallout3Root}");

        if (!File.Exists(Path.Combine(Fallout3Root, "Fallout3.exe")))
            throw new FileNotFoundException("Fallout3.exe not found in Fallout 3 directory");

        if (string.IsNullOrWhiteSpace(FalloutNVRoot))
            throw new InvalidOperationException("Fallout New Vegas root path is required");

        if (!Directory.Exists(FalloutNVRoot))
            throw new DirectoryNotFoundException($"Fallout New Vegas directory not found: {FalloutNVRoot}");

        if (!File.Exists(Path.Combine(FalloutNVRoot, "FalloutNV.exe")))
            throw new FileNotFoundException("FalloutNV.exe not found in Fallout New Vegas directory");

        if (string.IsNullOrWhiteSpace(DestinationPath))
            throw new InvalidOperationException("Destination path is required");

        if (string.IsNullOrWhiteSpace(MpiPackagePath))
            throw new InvalidOperationException("MPI package path is required");

        // Accept either .mpi file or extracted directory
        bool isMpiFile = File.Exists(MpiPackagePath) && MpiPackagePath.EndsWith(".mpi", StringComparison.OrdinalIgnoreCase);
        bool isDirectory = Directory.Exists(MpiPackagePath);

        if (!isMpiFile && !isDirectory)
            throw new FileNotFoundException($"MPI package not found: {MpiPackagePath}");

        // If it's a directory (already extracted), validate index.json exists
        if (isDirectory && !File.Exists(Path.Combine(MpiPackagePath, "_package", "index.json")))
            throw new FileNotFoundException("index.json not found in MPI package directory (_package/index.json)");
    }

    /// <summary>
    /// Whether to start the installation (must pass --start flag)
    /// </summary>
    public bool StartInstallation { get; set; }

    /// <summary>
    /// Create configuration from command-line arguments
    /// </summary>
    public static InstallConfig FromArgs(string[] args)
    {
        var config = new InstallConfig();

        for (int i = 0; i < args.Length; i++)
        {
            switch (args[i])
            {
                case "--fo3" when i + 1 < args.Length:
                    config.Fallout3Root = args[++i];
                    break;
                case "--fnv" when i + 1 < args.Length:
                    config.FalloutNVRoot = args[++i];
                    break;
                case "--output" or "-o" when i + 1 < args.Length:
                    config.DestinationPath = args[++i];
                    break;
                case "--mpi" when i + 1 < args.Length:
                    config.MpiPackagePath = args[++i];
                    break;
                case "--start":
                    config.StartInstallation = true;
                    break;
                case "--help" or "-h":
                    PrintHelp();
                    Environment.Exit(0);
                    break;
            }
        }

        return config;
    }

    /// <summary>
    /// Load configuration from a JSON file
    /// </summary>
    public static InstallConfig? FromFile(string configPath)
    {
        if (!File.Exists(configPath))
            return null;

        try
        {
            var json = File.ReadAllText(configPath);
            return System.Text.Json.JsonSerializer.Deserialize<InstallConfig>(json);
        }
        catch
        {
            return null;
        }
    }

    /// <summary>
    /// Save configuration to a JSON file
    /// </summary>
    public void SaveToFile(string configPath)
    {
        var json = System.Text.Json.JsonSerializer.Serialize(this, new System.Text.Json.JsonSerializerOptions
        {
            WriteIndented = true
        });
        File.WriteAllText(configPath, json);
    }

    public void PrintConfiguration()
    {
        Console.WriteLine("Installation Configuration:");
        Console.WriteLine($"  Fallout 3:   {Fallout3Root}");
        Console.WriteLine($"  Fallout NV:  {FalloutNVRoot}");
        Console.WriteLine($"  Output:      {DestinationPath}");
        Console.WriteLine($"  MPI Package: {MpiPackagePath}");
    }

    private static void PrintHelp()
    {
        Console.WriteLine("TTW Installer - Tale of Two Wastelands Installation Tool");
        Console.WriteLine();
        Console.WriteLine("Usage:");
        Console.WriteLine("  ttw-installer [options] --start");
        Console.WriteLine();
        Console.WriteLine("Options:");
        Console.WriteLine("  --fo3 <path>        Path to Fallout 3 installation directory");
        Console.WriteLine("  --fnv <path>        Path to Fallout New Vegas installation directory");
        Console.WriteLine("  --mpi <path>        Path to extracted TTW MPI package");
        Console.WriteLine("  --output <path>     Output directory for TTW installation");
        Console.WriteLine("  -o <path>           Alias for --output");
        Console.WriteLine("  --start             Start the installation (REQUIRED)");
        Console.WriteLine("  --help, -h          Show this help message");
        Console.WriteLine();
        Console.WriteLine("Example:");
        Console.WriteLine("  ttw-installer --fo3 ~/.steam/steam/steamapps/common/\"Fallout 3 goty\" \\");
        Console.WriteLine("                --fnv ~/.steam/steam/steamapps/common/\"Fallout New Vegas\" \\");
        Console.WriteLine("                --mpi ~/Downloads/TaleOfTwoWastelands-v3.3.2 \\");
        Console.WriteLine("                --output ~/TTW \\");
        Console.WriteLine("                --start");
        Console.WriteLine();
        Console.WriteLine("Configuration File:");
        Console.WriteLine("  You can also create a 'ttw-config.json' file in the current directory with:");
        Console.WriteLine("  {");
        Console.WriteLine("    \"Fallout3Root\": \"/path/to/fo3\",");
        Console.WriteLine("    \"FalloutNVRoot\": \"/path/to/fnv\",");
        Console.WriteLine("    \"MpiPackagePath\": \"/path/to/mpi\",");
        Console.WriteLine("    \"DestinationPath\": \"/path/to/output\"");
        Console.WriteLine("  }");
        Console.WriteLine();
        Console.WriteLine("  Then run: ttw-installer --start");
    }
}
