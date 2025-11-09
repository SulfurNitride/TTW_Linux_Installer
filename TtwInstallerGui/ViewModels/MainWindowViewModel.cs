using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using Avalonia.Controls;
using Avalonia.Platform.Storage;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using TtwInstaller.Models;
using TtwInstaller.Services;

namespace TtwInstallerGui.ViewModels;

public partial class MainWindowViewModel : ObservableObject
{
    // Universal MPI Installer Properties
    [ObservableProperty]
    private string _universalMpiPath = "";

    [ObservableProperty]
    private string _universalFo3Path = "";

    [ObservableProperty]
    private string _universalFnvPath = "";

    [ObservableProperty]
    private string _universalOblivionPath = "";

    [ObservableProperty]
    private string _universalOutputPath = "";

    [ObservableProperty]
    private bool _universalNeedsFo3 = false;

    [ObservableProperty]
    private bool _universalNeedsFnv = false;

    [ObservableProperty]
    private bool _universalNeedsOblivion = false;

    [ObservableProperty]
    private bool _universalNeedsOutput = false;

    [ObservableProperty]
    private string _universalMpiInfo = "Select an MPI file to begin...";

    [ObservableProperty]
    private bool _universalCreateBsas = true;

    [ObservableProperty]
    private bool _universalShowBsaOption = false;

    [ObservableProperty]
    private double _universalProgressPercent = 0;

    [ObservableProperty]
    private string _universalProgressText = "Ready";

    [ObservableProperty]
    private string _universalLogOutput = "";

    [ObservableProperty]
    private string _universalInstallButtonText = "Install MPI";

    [ObservableProperty]
    private bool _universalMpiLoaded = false;

    [ObservableProperty]
    private bool _isInstalling = false;

    private StringBuilder _universalLogBuilder = new StringBuilder();

    // Cached universal MPI data
    private TtwManifest? _universalCachedManifest;
    private string? _universalCachedExtractedPath;

    private Window? _mainWindow;

    public void SetMainWindow(Window window)
    {
        _mainWindow = window;
    }

    // ========== Universal MPI Installer Methods ==========

    [RelayCommand]
    private async Task UniversalBrowseMpi()
    {
        var path = await BrowseMpiFile("Select MPI Package");
        if (!string.IsNullOrEmpty(path))
        {
            UniversalMpiPath = path;
            await ParseUniversalMpi();
        }
    }

    [RelayCommand]
    private async Task UniversalBrowseFo3()
    {
        var path = await BrowseFolder("Select Fallout 3 Data Folder or Root Folder");
        if (!string.IsNullOrEmpty(path))
        {
            UniversalFo3Path = path;
        }
    }

    [RelayCommand]
    private async Task UniversalBrowseFnv()
    {
        var path = await BrowseFolder("Select Fallout New Vegas Data Folder or Root Folder");
        if (!string.IsNullOrEmpty(path))
        {
            UniversalFnvPath = path;
        }
    }

    [RelayCommand]
    private async Task UniversalBrowseOblivion()
    {
        var path = await BrowseFolder("Select Oblivion Data Folder or Root Folder");
        if (!string.IsNullOrEmpty(path))
        {
            UniversalOblivionPath = path;
        }
    }

    [RelayCommand]
    private async Task UniversalBrowseOutput()
    {
        var path = await BrowseFolder("Select Output Folder");
        if (!string.IsNullOrEmpty(path))
        {
            UniversalOutputPath = path;
        }
    }

    private async Task ParseUniversalMpi()
    {
        try
        {
            UniversalMpiInfo = "Extracting MPI package...";
            UniversalMpiLoaded = false;

            await Task.Run(() =>
            {
                // Always extract the .mpi file
                var extractedPath = MpiExtractor.ExtractMpiToTemp(UniversalMpiPath);
                _universalCachedExtractedPath = extractedPath;

                string packagePath = extractedPath;

                // Load manifest
                var loader = new ManifestLoader();
                var manifestPath = Path.Combine(packagePath, "_package", "index.json");

                if (!File.Exists(manifestPath))
                {
                    throw new Exception($"No manifest found at: {manifestPath}");
                }

                var manifest = loader.LoadFromFile(manifestPath);
                _universalCachedManifest = manifest;

                // Parse assets to get counts
                var assets = loader.ParseAssets(manifest);

                // Build MPI info display
                var infoBuilder = new StringBuilder();
                infoBuilder.AppendLine($"📦 {manifest.Package?.Title ?? "Unknown Package"}");
                infoBuilder.AppendLine($"Version: {manifest.Package?.Version ?? "Unknown"}");
                infoBuilder.AppendLine($"Author: {manifest.Package?.Author ?? "Unknown"}");
                if (!string.IsNullOrEmpty(manifest.Package?.Description))
                {
                    infoBuilder.AppendLine($"\n{manifest.Package.Description}");
                }
                infoBuilder.AppendLine();

                // Asset summary
                infoBuilder.AppendLine($"Total Assets: {assets.Count}");
                var grouped = assets.GroupBy(a => a.OpType).OrderBy(g => g.Key);
                foreach (var group in grouped)
                {
                    string opName = group.Key switch
                    {
                        0 => "Copy",
                        1 => "New",
                        2 => "Patch (xdelta3)",
                        3 => "Unknown",
                        4 => "Audio (XWM)",
                        5 => "Audio (OGG)",
                        _ => $"Type {group.Key}"
                    };
                    infoBuilder.AppendLine($"  • {opName}: {group.Count()} files");
                }

                UniversalMpiInfo = infoBuilder.ToString();

                // Detect required paths from all profiles
                DetectRequiredPaths(manifest, loader);

                // Check if this MPI creates BSAs
                UniversalShowBsaOption = manifest.Package?.Title?.Contains("NMC") == true;

                UniversalMpiLoaded = true;
            });
        }
        catch (Exception ex)
        {
            UniversalMpiInfo = $"❌ Error parsing MPI:\n{ex.Message}";
            UniversalMpiLoaded = false;
        }
    }

    private void DetectRequiredPaths(TtwManifest manifest, ManifestLoader loader)
    {
        // Reset all flags
        UniversalNeedsFo3 = false;
        UniversalNeedsFnv = false;
        UniversalNeedsOblivion = false;
        UniversalNeedsOutput = false;

        // Check all profiles to detect what paths are needed
        if (manifest.Variables != null)
        {
            foreach (var profileVars in manifest.Variables)
            {
                foreach (var variable in profileVars)
                {
                    string value = variable.Value?.ToUpper() ?? "";

                    if (value.Contains("FO3") || value.Contains("FALLOUT3"))
                        UniversalNeedsFo3 = true;

                    if (value.Contains("FNV") || value.Contains("FALLOUTNV"))
                        UniversalNeedsFnv = true;

                    if (value.Contains("TES4") || value.Contains("OBLIVION"))
                        UniversalNeedsOblivion = true;

                    if (value.Contains("DESTINATION"))
                        UniversalNeedsOutput = true;
                }
            }
        }

        // Always show output path
        UniversalNeedsOutput = true;
    }

    [RelayCommand]
    private async Task StartUniversalInstall()
    {
        if (IsInstalling) return;

        // Validate
        if (_universalCachedManifest == null)
        {
            AppendUniversalLog("❌ Error: No MPI loaded. Please select an MPI file first.");
            return;
        }

        // Check required paths
        if (UniversalNeedsFo3 && string.IsNullOrWhiteSpace(UniversalFo3Path))
        {
            AppendUniversalLog("❌ Error: Fallout 3 path is required!");
            return;
        }
        if (UniversalNeedsFnv && string.IsNullOrWhiteSpace(UniversalFnvPath))
        {
            AppendUniversalLog("❌ Error: Fallout New Vegas path is required!");
            return;
        }
        if (UniversalNeedsOblivion && string.IsNullOrWhiteSpace(UniversalOblivionPath))
        {
            AppendUniversalLog("❌ Error: Oblivion path is required!");
            return;
        }
        if (string.IsNullOrWhiteSpace(UniversalOutputPath))
        {
            AppendUniversalLog("❌ Error: Output path is required!");
            return;
        }

        IsInstalling = true;
        UniversalInstallButtonText = "Installing...";
        _universalLogBuilder.Clear();
        UniversalLogOutput = "";
        UniversalProgressPercent = 0;
        UniversalProgressText = "Starting...";

        try
        {
            await Task.Run(() => RunUniversalInstallInternal());
            AppendUniversalLog("\n✅ Installation completed successfully!");
        }
        catch (Exception ex)
        {
            AppendUniversalLog($"\n❌ Error: {ex.Message}");
            AppendUniversalLog($"{ex.StackTrace}");
        }
        finally
        {
            IsInstalling = false;
            UniversalInstallButtonText = "Install MPI";
        }
    }

    private void RunUniversalInstallInternal()
    {
        if (_universalCachedManifest == null || _universalCachedExtractedPath == null)
        {
            throw new Exception("No manifest loaded");
        }

        AppendUniversalLog($"=== {_universalCachedManifest.Package?.Title ?? "MPI Installation"} ===\n");

        var loader = new ManifestLoader();
        string packagePath = _universalCachedExtractedPath;

        // Use profile 1 (game data profile) but override destination to user's custom output
        // Profile 1 typically has the correct location mappings for game paths
        int profileIndex = 1;
        var locations = loader.GetLocations(_universalCachedManifest, profileIndex);
        var assets = loader.ParseAssets(_universalCachedManifest);

        AppendUniversalLog($"Assets to process: {assets.Count}");
        AppendUniversalLog($"Locations: {locations.Count}\n");

        // Normalize paths
        string? fo3Root = UniversalNeedsFo3 && !string.IsNullOrWhiteSpace(UniversalFo3Path)
            ? NormalizeGamePath(UniversalFo3Path) : null;
        string? fnvRoot = UniversalNeedsFnv && !string.IsNullOrWhiteSpace(UniversalFnvPath)
            ? NormalizeGamePath(UniversalFnvPath) : null;
        string? oblivionRoot = UniversalNeedsOblivion && !string.IsNullOrWhiteSpace(UniversalOblivionPath)
            ? NormalizeGamePath(UniversalOblivionPath) : null;

        if (fo3Root != null) AppendUniversalLog($"FO3 Root: {fo3Root}");
        if (fnvRoot != null) AppendUniversalLog($"FNV Root: {fnvRoot}");
        if (oblivionRoot != null) AppendUniversalLog($"Oblivion Root: {oblivionRoot}");
        AppendUniversalLog($"Output: {UniversalOutputPath}\n");

        // Build config
        var config = new InstallConfig
        {
            Fallout3Root = fo3Root ?? "",
            FalloutNVRoot = fnvRoot ?? "",
            OblivionRoot = oblivionRoot ?? "",
            DestinationPath = UniversalOutputPath,
            MpiPackagePath = packagePath
        };

        // Create services
        var locationResolver = new LocationResolver(locations, config);
        using var bsaReader = new BsaReader();
        var logger = new InstallationLogger();

        // Check if we need BSA creation (for NMC and similar packages)
        BsaWriter? bsaWriter = null;
        if (UniversalShowBsaOption && UniversalCreateBsas)
        {
            bsaWriter = new BsaWriter(locations, UniversalOutputPath);
        }

        var assetProcessor = new AssetProcessor(locationResolver, bsaReader, config, bsaWriter, logger);

        // Filter assets by OpType for optimal parallel processing
        var copyAssets = new List<Asset>();
        var newAssets = new List<Asset>();
        var patchAssets = new List<Asset>();
        var oggEnc2Assets = new List<Asset>();
        var audioEncAssets = new List<Asset>();

        foreach (var asset in assets)
        {
            switch (asset.OpType)
            {
                case 0: copyAssets.Add(asset); break;
                case 1: newAssets.Add(asset); break;
                case 2: patchAssets.Add(asset); break;
                case 4: oggEnc2Assets.Add(asset); break;
                case 5: audioEncAssets.Add(asset); break;
            }
        }

        int totalAssets = newAssets.Count + copyAssets.Count + patchAssets.Count +
                         oggEnc2Assets.Count + audioEncAssets.Count;
        int processedAssets = 0;
        int successCount = 0;
        int errorCount = 0;

        // Create destination
        Directory.CreateDirectory(UniversalOutputPath);

        // Process New assets (15-20%) - Parallelized
        UpdateUniversalProgress(15, $"Processing {newAssets.Count} new assets...");
        int newProcessed = 0;
        var parallelOptions = new ParallelOptions { MaxDegreeOfParallelism = 4 };
        Parallel.ForEach(newAssets, parallelOptions, asset =>
        {
            bool success = assetProcessor.ProcessAsset(asset);
            int current = Interlocked.Increment(ref newProcessed);
            Interlocked.Increment(ref processedAssets);
            if (success) Interlocked.Increment(ref successCount);
            else Interlocked.Increment(ref errorCount);

            if (current % 100 == 0)
            {
                double percent = 15 + (5.0 * current / newAssets.Count);
                UpdateUniversalProgress(percent, $"New assets: {current}/{newAssets.Count}");
            }
        });

        // Process Copy assets (20-50%) - Parallelized
        UpdateUniversalProgress(20, $"Processing {copyAssets.Count} copy operations...");
        int copyCount = 0;
        Parallel.ForEach(copyAssets, parallelOptions, asset =>
        {
            bool success = assetProcessor.ProcessAsset(asset);
            int current = Interlocked.Increment(ref copyCount);
            Interlocked.Increment(ref processedAssets);
            if (success) Interlocked.Increment(ref successCount);
            else Interlocked.Increment(ref errorCount);

            if (current % 500 == 0)
            {
                double percent = 20 + (30.0 * current / copyAssets.Count);
                UpdateUniversalProgress(percent, $"Copy: {current}/{copyAssets.Count}");
            }
        });

        // Process Patch assets (50-60%) - Parallelized
        UpdateUniversalProgress(50, $"Processing {patchAssets.Count} patches...");
        int patchCount = 0;
        Parallel.ForEach(patchAssets, parallelOptions, asset =>
        {
            bool success = assetProcessor.ProcessAsset(asset);
            int current = Interlocked.Increment(ref patchCount);
            Interlocked.Increment(ref processedAssets);
            if (success) Interlocked.Increment(ref successCount);
            else Interlocked.Increment(ref errorCount);

            if (current % 100 == 0)
            {
                double percent = 50 + (10.0 * current / patchAssets.Count);
                UpdateUniversalProgress(percent, $"Patches: {current}/{patchAssets.Count}");
            }
        });

        // Detect CPU cores for parallel audio processing - USE ALL CORES!
        var cpuCores = Environment.ProcessorCount;
        var parallelThreads = cpuCores; // Use ALL cores for ffmpeg - 100% CPU!
        AppendUniversalLog($"\nDetected {cpuCores} CPU cores, using {parallelThreads} threads for parallel audio processing (100% CPU)\n");

        // Process OggEnc2 assets (60-75%) - PARALLEL
        UpdateUniversalProgress(60, $"Processing {oggEnc2Assets.Count} OggEnc2 operations...");
        int oggCount = 0;
        object oggLock = new object();

        Parallel.ForEach(oggEnc2Assets,
            new ParallelOptions { MaxDegreeOfParallelism = parallelThreads },
            asset =>
            {
                bool success = assetProcessor.ProcessAsset(asset);

                lock (oggLock)
                {
                    oggCount++;
                    processedAssets++;
                    if (success) successCount++;
                    else errorCount++;

                    if (oggCount % 1000 == 0)
                    {
                        double percent = 60 + (15.0 * oggCount / oggEnc2Assets.Count);
                        UpdateUniversalProgress(percent, $"OggEnc2: {oggCount}/{oggEnc2Assets.Count}");
                    }
                }
            });

        // Process AudioEnc assets (75-80%) - PARALLEL
        UpdateUniversalProgress(75, $"Processing {audioEncAssets.Count} AudioEnc operations...");
        int audioCount = 0;
        object audioLock = new object();

        Parallel.ForEach(audioEncAssets,
            new ParallelOptions { MaxDegreeOfParallelism = parallelThreads },
            asset =>
            {
                bool success = assetProcessor.ProcessAsset(asset);

                lock (audioLock)
                {
                    audioCount++;
                    processedAssets++;
                    if (success) successCount++;
                    else errorCount++;

                    if (audioCount % 100 == 0)
                    {
                        double percent = 75 + (5.0 * audioCount / audioEncAssets.Count);
                        UpdateUniversalProgress(percent, $"AudioEnc: {audioCount}/{audioEncAssets.Count}");
                    }
                }
            });

        // Write BSAs (80-95%)
        if (bsaWriter != null)
        {
            UpdateUniversalProgress(80, "Writing BSA archives...");
            AppendUniversalLog("\n=== Writing BSA Archives ===");
            int bsaFailCount = bsaWriter.WriteAllBsas();

            if (bsaFailCount > 0)
            {
                AppendUniversalLog($"⚠️ Failed to write {bsaFailCount} BSA files");
            }
            else
            {
                AppendUniversalLog("✅ All BSA archives written successfully");
            }
        }

        // Run post-commands (95-100%)
        UpdateUniversalProgress(95, "Running post-installation commands...");
        AppendUniversalLog("\n=== Running Post-Installation Commands ===");

        var postCommandRunner = new PostCommandRunner(UniversalOutputPath, config);
        var postCommands = _universalCachedManifest?.PostCommands ?? new List<PostCommand>();
        postCommandRunner.RunPostCommands(postCommands);

        UpdateUniversalProgress(100, "Installation complete!");
        AppendUniversalLog($"\n✅ Assets processed: {processedAssets}/{totalAssets}");
        AppendUniversalLog($"   Success: {successCount}, Errors: {errorCount}");

        // Write log
        logger.WriteLogFile(UniversalOutputPath);
        AppendUniversalLog($"\n📝 Detailed log saved to: {Path.Combine(UniversalOutputPath, "ttw-installation.log")}");
    }

    private void AppendUniversalLog(string message)
    {
        _universalLogBuilder.AppendLine(message);
        UniversalLogOutput = _universalLogBuilder.ToString();
    }

    private void UpdateUniversalProgress(double percent, string text)
    {
        UniversalProgressPercent = percent;
        UniversalProgressText = text;
    }

    // ========== Helper Methods ==========

    private async Task<string> BrowseFolder(string title)
    {
        if (_mainWindow == null) return "";

        var folders = await _mainWindow.StorageProvider.OpenFolderPickerAsync(new FolderPickerOpenOptions
        {
            Title = title,
            AllowMultiple = false
        });

        return folders.Count > 0 ? folders[0].Path.LocalPath : "";
    }

    private async Task<string> BrowseMpiFile(string title)
    {
        if (_mainWindow == null) return "";

        var files = await _mainWindow.StorageProvider.OpenFilePickerAsync(new FilePickerOpenOptions
        {
            Title = title,
            AllowMultiple = false,
            FileTypeFilter = new[]
            {
                new FilePickerFileType("MPI Package") { Patterns = new[] { "*.mpi" } },
                new FilePickerFileType("All Files") { Patterns = new[] { "*" } }
            }
        });

        return files.Count > 0 ? files[0].Path.LocalPath : "";
    }

    private static string NormalizeGamePath(string inputPath)
    {
        // Trim trailing slashes first, or Path.GetDirectoryName won't work correctly
        string cleanedPath = inputPath.TrimEnd(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar);

        // Determine if user provided Data folder or game root
        if (Path.GetFileName(cleanedPath).Equals("Data", StringComparison.OrdinalIgnoreCase))
        {
            // User provided the Data folder - use parent directory as root
            return Path.GetDirectoryName(cleanedPath) ?? cleanedPath;
        }
        else
        {
            // User provided the game root folder
            return cleanedPath;
        }
    }
}
