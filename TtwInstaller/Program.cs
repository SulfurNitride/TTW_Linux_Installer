using TtwInstaller.Models;
using TtwInstaller.Services;

namespace TtwInstaller;

public static class Program
{
    public static int Main(string[] args)
    {
        // Try multiple config file names for compatibility
        var configFile = "mpi-config.json";
        if (!File.Exists(configFile))
        {
            configFile = "ttw-config.json"; // Legacy compatibility
        }

        var config = InstallConfig.FromFile(configFile);

        if (config == null)
        {
            config = InstallConfig.FromArgs(args);

            // Only MPI package and output are truly required (games are optional based on manifest)
            bool missingRequired = string.IsNullOrWhiteSpace(config.MpiPackagePath) ||
                                  string.IsNullOrWhiteSpace(config.DestinationPath);

            if (missingRequired)
            {
                Console.WriteLine("❌ Missing required paths. Please provide configuration via:");
                Console.WriteLine("   1. Command-line arguments (run with --help for details)");
                Console.WriteLine("   2. Create a mpi-config.json file in the current directory");
                Console.WriteLine();
                InstallConfig.FromArgs(new[] { "--help" });
                return 1;
            }
        }
        else
        {
            Console.WriteLine($"✅ Loaded configuration from {configFile}\n");
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
                Console.WriteLine("  ./TtwInstaller --mpi <path> --output <path> [--fo3 <path>] [--fnv <path>] [--oblivion <path>] --start");
            }
            Console.WriteLine();
            Console.WriteLine("Or run with --help for more information.");
            return 0;
        }

        Console.WriteLine("🚀 Starting installation...\n");

        return RunInstallation(config);
    }

    public static int RunInstallation(InstallConfig config)
    {
        Console.WriteLine("=== Universal MPI Installer ===\n");

        Console.WriteLine("Validating configuration...");
        try
        {
            config.Validate();
            config.PrintConfiguration();
            Console.WriteLine("✅ Configuration valid\n");
        }
        catch (Exception ex)
        {
            Console.WriteLine($"❌ Configuration error: {ex.Message}");
            return 1;
        }

        // Check if MPI extraction is needed
        string? extractedMpiPath = null;
        if (MpiExtractor.IsMpiFile(config.MpiPackagePath))
        {
            Console.WriteLine("Detected .mpi file - extracting...");
            try
            {
                extractedMpiPath = MpiExtractor.ExtractMpiToTemp(config.MpiPackagePath);
                config.MpiPackagePath = extractedMpiPath; // Update config to use extracted path
            }
            catch (Exception ex)
            {
                Console.WriteLine($"❌ MPI extraction failed: {ex.Message}");
                return 1;
            }
        }

        try
        {
            return RunInstallationInternal(config);
        }
        finally
        {
            // Clean up extracted MPI if we created one
            if (extractedMpiPath != null)
            {
                MpiExtractor.CleanupTempDirectory(extractedMpiPath);
            }
        }
    }

    private static int RunInstallationInternal(InstallConfig config)
    {
        // Load manifest
        Console.WriteLine("Loading manifest...");
        var loader = new ManifestLoader();
        var manifestPath = Path.Combine(config.MpiPackagePath, "_package", "index.json");
        var manifest = loader.LoadFromFile(manifestPath);

        // Get locations (profile 1 = installation profile)
        var locations = loader.GetLocations(manifest, 1);
        Console.WriteLine($"Loaded {locations.Count} locations");

        // Parse assets
        var assets = loader.ParseAssets(manifest);
        Console.WriteLine($"Loaded {assets.Count} total assets");

        // Get checks for validation
        var checks = manifest.Checks;
        Console.WriteLine($"Loaded {checks?.Count ?? 0} validation checks\n");

        // Create services
        var locationResolver = new LocationResolver(locations, config);

        // Run pre-flight validation checks
        var validationService = new ValidationService(config, locationResolver);
        bool validationPassed = validationService.RunValidationChecks(checks);

        if (!validationPassed)
        {
            Console.WriteLine("❌ Validation failed. Please fix the issues above before continuing.");
            Console.Write("\nContinue anyway? (yes/no): ");
            string? response = Console.ReadLine();

            if (response?.Trim().ToLower() != "yes")
            {
                Console.WriteLine("Installation aborted.");
                return 1;
            }

            Console.WriteLine("\n⚠️  Continuing despite validation failures...\n");
        }

        using var bsaReader = new BsaReader();

        // Create BSA writer for packing files into archives
        using var bsaWriter = new BsaWriter(locations, config.DestinationPath);

        var assetProcessor = new AssetProcessor(locationResolver, bsaReader, config, bsaWriter);

        // Filter assets for installation (Copy, New, Patch, and Audio operations)
        var copyAssets = assets.Where(a => a.OpType == 0).ToList();
        var newAssets = assets.Where(a => a.OpType == 1).ToList();
        var patchAssets = assets.Where(a => a.OpType == 2).ToList();
        var oggEnc2Assets = assets.Where(a => a.OpType == 4).ToList();
        var audioEncAssets = assets.Where(a => a.OpType == 5).ToList();

        Console.WriteLine($"Processing {newAssets.Count} New, {copyAssets.Count} Copy, {patchAssets.Count} Patch, {oggEnc2Assets.Count} OggEnc2, and {audioEncAssets.Count} AudioEnc operations\n");

        // Create destination directory
        Directory.CreateDirectory(config.DestinationPath);
        Console.WriteLine($"Created destination: {config.DestinationPath}\n");

        // Process New assets first (these are needed files like DLLs)
        Console.WriteLine("--- Processing New Assets ---");
        int newSuccessCount = 0;
        int newFailCount = 0;

        foreach (var asset in newAssets)
        {
            try
            {
                Console.Write($"  [{newSuccessCount + newFailCount + 1}/{newAssets.Count}] {asset.SourcePath} ... ");

                if (assetProcessor.ProcessAsset(asset))
                {
                    Console.WriteLine("✅");
                    newSuccessCount++;
                }
                else
                {
                    Console.WriteLine("❌");
                    newFailCount++;
                }
            }
            catch (Exception ex)
            {
                Console.WriteLine($"❌ {ex.Message}");
                newFailCount++;
            }
        }

        Console.WriteLine($"\nNew assets: {newSuccessCount} succeeded, {newFailCount} failed\n");

        // Process Copy assets
        Console.WriteLine("--- Processing Copy Assets ---");
        int copySuccessCount = 0;
        int copyFailCount = 0;

        foreach (var asset in copyAssets)
        {
            try
            {
                var sourceLocation = locationResolver.GetLocation(asset.SourceLoc);

                Console.Write($"  [{copySuccessCount + copyFailCount + 1}/{copyAssets.Count}] {asset.SourcePath}");
                Console.Write($" (from {sourceLocation.Name}) ... ");

                if (assetProcessor.ProcessAsset(asset))
                {
                    Console.WriteLine("✅");
                    copySuccessCount++;
                }
                else
                {
                    Console.WriteLine("❌");
                    copyFailCount++;
                }
            }
            catch (Exception ex)
            {
                Console.WriteLine($"❌ {ex.Message}");
                copyFailCount++;
            }
        }

        Console.WriteLine($"\nCopy assets: {copySuccessCount} succeeded, {copyFailCount} failed\n");

        // Process Patch assets
        Console.WriteLine("--- Processing Patch Assets ---");
        int patchSuccessCount = 0;
        int patchFailCount = 0;

        foreach (var asset in patchAssets)
        {
            try
            {
                var sourceLocation = locationResolver.GetLocation(asset.SourceLoc);

                Console.Write($"  [{patchSuccessCount + patchFailCount + 1}/{patchAssets.Count}] {asset.SourcePath}");
                Console.Write($" (from {sourceLocation.Name}) ... ");

                if (assetProcessor.ProcessAsset(asset))
                {
                    Console.WriteLine("✅");
                    patchSuccessCount++;
                }
                else
                {
                    Console.WriteLine("❌");
                    patchFailCount++;
                }
            }
            catch (Exception ex)
            {
                Console.WriteLine($"❌ {ex.Message}");
                patchFailCount++;
            }
        }

        Console.WriteLine($"\nPatch assets: {patchSuccessCount} succeeded, {patchFailCount} failed\n");

        // Detect CPU cores for parallel processing
        var cpuCores = Environment.ProcessorCount;
        var parallelThreads = Math.Max(1, cpuCores - 2); // Leave 2 cores for system
        Console.WriteLine($"Detected {cpuCores} CPU cores, using {parallelThreads} threads for parallel processing\n");

        // Process OggEnc2 assets (audio resampling) - PARALLEL
        Console.WriteLine("--- Processing OggEnc2 Assets (Parallel) ---");
        int oggEnc2SuccessCount = 0;
        int oggEnc2FailCount = 0;
        object oggEnc2Lock = new object();

        Parallel.ForEach(oggEnc2Assets,
            new ParallelOptions { MaxDegreeOfParallelism = parallelThreads },
            asset =>
        {
            try
            {
                var sourceLocation = locationResolver.GetLocation(asset.SourceLoc);

                int currentIndex;
                lock (oggEnc2Lock)
                {
                    currentIndex = oggEnc2SuccessCount + oggEnc2FailCount + 1;
                }

                var success = assetProcessor.ProcessAsset(asset);

                lock (oggEnc2Lock)
                {
                    Console.Write($"  [{currentIndex}/{oggEnc2Assets.Count}] {asset.SourcePath}");
                    Console.Write($" (from {sourceLocation.Name}) ... ");

                    if (success)
                    {
                        Console.WriteLine("✅");
                        oggEnc2SuccessCount++;
                    }
                    else
                    {
                        Console.WriteLine("❌");
                        oggEnc2FailCount++;
                    }
                }
            }
            catch (Exception ex)
            {
                lock (oggEnc2Lock)
                {
                    Console.WriteLine($"  [{oggEnc2SuccessCount + oggEnc2FailCount + 1}/{oggEnc2Assets.Count}] {asset.SourcePath} ❌ {ex.Message}");
                    oggEnc2FailCount++;
                }
            }
        });

        Console.WriteLine($"\nOggEnc2 assets: {oggEnc2SuccessCount} succeeded, {oggEnc2FailCount} failed\n");

        // Process AudioEnc assets (audio format conversion) - PARALLEL
        Console.WriteLine("--- Processing AudioEnc Assets (Parallel) ---");
        int audioEncSuccessCount = 0;
        int audioEncFailCount = 0;
        object audioEncLock = new object();

        Parallel.ForEach(audioEncAssets,
            new ParallelOptions { MaxDegreeOfParallelism = parallelThreads },
            asset =>
        {
            try
            {
                var sourceLocation = locationResolver.GetLocation(asset.SourceLoc);

                int currentIndex;
                lock (audioEncLock)
                {
                    currentIndex = audioEncSuccessCount + audioEncFailCount + 1;
                }

                var success = assetProcessor.ProcessAsset(asset);

                lock (audioEncLock)
                {
                    Console.Write($"  [{currentIndex}/{audioEncAssets.Count}] {asset.SourcePath}");
                    Console.Write($" (from {sourceLocation.Name}) ... ");

                    if (success)
                    {
                        Console.WriteLine("✅");
                        audioEncSuccessCount++;
                    }
                    else
                    {
                        Console.WriteLine("❌");
                        audioEncFailCount++;
                    }
                }
            }
            catch (Exception ex)
            {
                lock (audioEncLock)
                {
                    Console.WriteLine($"  [{audioEncSuccessCount + audioEncFailCount + 1}/{audioEncAssets.Count}] {asset.SourcePath} ❌ {ex.Message}");
                    audioEncFailCount++;
                }
            }
        });

        Console.WriteLine($"\nAudioEnc assets: {audioEncSuccessCount} succeeded, {audioEncFailCount} failed\n");

        // Write BSA archives
        int bsaFailCount = bsaWriter.WriteAllBsas();

        // Run post-installation commands (file renaming, etc.)
        var postCommandRunner = new PostCommandRunner(config.DestinationPath);
        int postCommandFailCount = postCommandRunner.RunPostCommands(manifest.PostCommands);

        // Summary
        Console.WriteLine("\n=== Installation Summary ===");
        Console.WriteLine($"Assets processed: {newSuccessCount + copySuccessCount + patchSuccessCount + oggEnc2SuccessCount + audioEncSuccessCount}/{newAssets.Count + copyAssets.Count + patchAssets.Count + oggEnc2Assets.Count + audioEncAssets.Count}");
        Console.WriteLine($"  New: {newSuccessCount}/{newAssets.Count}");
        Console.WriteLine($"  Copy: {copySuccessCount}/{copyAssets.Count}");
        Console.WriteLine($"  Patch: {patchSuccessCount}/{patchAssets.Count}");
        Console.WriteLine($"  OggEnc2: {oggEnc2SuccessCount}/{oggEnc2Assets.Count}");
        Console.WriteLine($"  AudioEnc: {audioEncSuccessCount}/{audioEncAssets.Count}");
        Console.WriteLine($"\nDestination: {config.DestinationPath}");

        int totalFails = newFailCount + copyFailCount + patchFailCount + oggEnc2FailCount + audioEncFailCount + bsaFailCount + postCommandFailCount;

        if (totalFails == 0)
        {
            Console.WriteLine("\n✅ Installation completed successfully!");
            Console.WriteLine("MPI package has been installed with all BSA archives created.");
        }
        else
        {
            Console.WriteLine($"\n⚠️  Installation completed with {totalFails} errors.");
        }

        return totalFails > 0 ? 1 : 0;
    }
}
