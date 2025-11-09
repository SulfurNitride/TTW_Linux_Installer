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

    private string? _fallout3DataOverride;

    /// <summary>
    /// Fallout 3 Data directory
    /// Can be overridden for special cases like BSA decompression
    /// </summary>
    public string Fallout3Data
    {
        get => _fallout3DataOverride ?? Path.Combine(Fallout3Root, "Data");
        set => _fallout3DataOverride = value;
    }

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
    /// Destination directory for MPI installation output
    /// </summary>
    public string DestinationPath { get; set; } = string.Empty;

    /// <summary>
    /// Path to extracted MPI package
    /// </summary>
    public string MpiPackagePath { get; set; } = string.Empty;

    /// <summary>
    /// Validate configuration
    /// Universal validation - only validates paths that are provided.
    /// The MPI manifest determines which game paths are actually needed.
    /// </summary>
    public void Validate()
    {
        // Validate Fallout 3 path if provided
        if (!string.IsNullOrWhiteSpace(Fallout3Root))
        {
            if (!Directory.Exists(Fallout3Root))
                throw new DirectoryNotFoundException($"Fallout 3 directory not found: {Fallout3Root}");

            if (!File.Exists(Path.Combine(Fallout3Root, "Fallout3.exe")))
                throw new FileNotFoundException($"Fallout3.exe not found in: {Fallout3Root}");
        }

        // Validate Fallout New Vegas path if provided
        if (!string.IsNullOrWhiteSpace(FalloutNVRoot))
        {
            if (!Directory.Exists(FalloutNVRoot))
                throw new DirectoryNotFoundException($"Fallout New Vegas directory not found: {FalloutNVRoot}");

            if (!File.Exists(Path.Combine(FalloutNVRoot, "FalloutNV.exe")))
                throw new FileNotFoundException($"FalloutNV.exe not found in: {FalloutNVRoot}");
        }

        // Validate Oblivion path if provided
        if (!string.IsNullOrWhiteSpace(OblivionRoot))
        {
            if (!Directory.Exists(OblivionRoot))
                throw new DirectoryNotFoundException($"Oblivion directory not found: {OblivionRoot}");

            if (!File.Exists(Path.Combine(OblivionRoot, "Oblivion.exe")))
                throw new FileNotFoundException($"Oblivion.exe not found in: {OblivionRoot}");
        }

        // Destination path is always required
        if (string.IsNullOrWhiteSpace(DestinationPath))
            throw new InvalidOperationException("Destination path is required");

        // MPI package path is always required
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
                case "--oblivion" when i + 1 < args.Length:
                    config.OblivionRoot = args[++i];
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
        Console.WriteLine("Universal MPI Installer - Installs any MPI package");
        Console.WriteLine();
        Console.WriteLine("Usage:");
        Console.WriteLine("  mpi-installer [options] --start");
        Console.WriteLine();
        Console.WriteLine("Options:");
        Console.WriteLine("  --fo3 <path>        Path to Fallout 3 installation directory (if needed)");
        Console.WriteLine("  --fnv <path>        Path to Fallout New Vegas installation directory (if needed)");
        Console.WriteLine("  --oblivion <path>   Path to Oblivion installation directory (if needed)");
        Console.WriteLine("  --mpi <path>        Path to MPI package file (.mpi) or extracted directory");
        Console.WriteLine("  --output <path>     Output directory for installation");
        Console.WriteLine("  -o <path>           Alias for --output");
        Console.WriteLine("  --start             Start the installation (REQUIRED)");
        Console.WriteLine("  --help, -h          Show this help message");
        Console.WriteLine();
        Console.WriteLine("Example (TTW):");
        Console.WriteLine("  mpi-installer --fo3 ~/.steam/steam/steamapps/common/\"Fallout 3 goty\" \\");
        Console.WriteLine("                --fnv ~/.steam/steam/steamapps/common/\"Fallout New Vegas\" \\");
        Console.WriteLine("                --mpi ~/Downloads/TaleOfTwoWastelands-v3.3.2.mpi \\");
        Console.WriteLine("                --output ~/TTW \\");
        Console.WriteLine("                --start");
        Console.WriteLine();
        Console.WriteLine("Example (Oblivion BSA Decompressor):");
        Console.WriteLine("  mpi-installer --oblivion ~/.steam/steam/steamapps/common/Oblivion \\");
        Console.WriteLine("                --mpi ~/Downloads/OblivionBSADecompressor.mpi \\");
        Console.WriteLine("                --output ~/OblivionDecompressed \\");
        Console.WriteLine("                --start");
        Console.WriteLine();
        Console.WriteLine("Configuration File:");
        Console.WriteLine("  You can also create a 'mpi-config.json' file in the current directory with:");
        Console.WriteLine("  {");
        Console.WriteLine("    \"Fallout3Root\": \"/path/to/fo3\",       // Optional");
        Console.WriteLine("    \"FalloutNVRoot\": \"/path/to/fnv\",      // Optional");
        Console.WriteLine("    \"OblivionRoot\": \"/path/to/oblivion\",  // Optional");
        Console.WriteLine("    \"MpiPackagePath\": \"/path/to/mpi\",     // Required");
        Console.WriteLine("    \"DestinationPath\": \"/path/to/output\"  // Required");
        Console.WriteLine("  }");
        Console.WriteLine();
        Console.WriteLine("  Then run: mpi-installer --start");
        Console.WriteLine();
        Console.WriteLine("Notes:");
        Console.WriteLine("  - The MPI manifest determines which game paths are actually required");
        Console.WriteLine("  - Only provide paths for games used by your specific MPI package");
    }
}
