using System;
using System.IO;
using System.Linq;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using System.Collections.Generic;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Layout;
using Avalonia.Media;
using Avalonia.Platform.Storage;
using Avalonia.Threading;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using TtwInstaller.Models;
using TtwInstaller.Services;

namespace TtwInstallerGui.ViewModels;

public partial class MainWindowViewModel : ViewModelBase
{
    // TTW Installer Properties
    [ObservableProperty]
    private string _fallout3Path = "";

    [ObservableProperty]
    private string _falloutNVPath = "";

    [ObservableProperty]
    private string _mpiPath = "";

    [ObservableProperty]
    private string _outputPath = "";

    [ObservableProperty]
    private double _progressPercent = 0;

    [ObservableProperty]
    private string _progressText = "Ready";

    [ObservableProperty]
    private string _logOutput = "";

    [ObservableProperty]
    private bool _isInstalling = false;

    [ObservableProperty]
    private string _installButtonText = "Start Installation";

    // BSA Decompressor Properties
    [ObservableProperty]
    private string _bsaFalloutNVPath = "";

    [ObservableProperty]
    private string _bsaMpiPath = "";

    [ObservableProperty]
    private string _bsaOutputPath = "";

    [ObservableProperty]
    private double _bsaProgressPercent = 0;

    [ObservableProperty]
    private string _bsaProgressText = "Ready";

    [ObservableProperty]
    private string _bsaLogOutput = "";

    [ObservableProperty]
    private string _bsaInstallButtonText = "Start Decompression";

    // Oblivion BSA Decompressor Properties
    [ObservableProperty]
    private string _oblivionDataPath = "";

    [ObservableProperty]
    private string _oblivionMpiPath = "";

    [ObservableProperty]
    private string _oblivionOutputPath = "";

    [ObservableProperty]
    private double _oblivionProgressPercent = 0;

    [ObservableProperty]
    private string _oblivionProgressText = "Ready";

    [ObservableProperty]
    private string _oblivionLogOutput = "";

    [ObservableProperty]
    private string _oblivionInstallButtonText = "Start Decompression";

    // Tab Control
    [ObservableProperty]
    private int _selectedTabIndex = 0;

    private StringBuilder _logBuilder = new StringBuilder();
    private StringBuilder _bsaLogBuilder = new StringBuilder();
    private StringBuilder _oblivionLogBuilder = new StringBuilder();
    private Window? _mainWindow;

    public void SetMainWindow(Window window)
    {
        _mainWindow = window;
        LoadConfig();
        LoadBsaConfig();
        LoadOblivionConfig();
        _ = ShowStartupDependencyCheckAsync();
    }

    private async Task ShowStartupDependencyCheckAsync()
    {
        await Task.Delay(500); // Small delay to let window fully render

        var (depsOk, missingDeps) = DependencyChecker.CheckDependencies();

        var message = new StringBuilder();
        message.AppendLine("=== System Requirements Check ===\n");

        // Check xdelta3
        try
        {
            var xdeltaPath = BundledBinaryManager.GetXdelta3Path();
            message.AppendLine("✅ xdelta3 (bundled) - Found");
        }
        catch
        {
            message.AppendLine("❌ xdelta3 (bundled) - MISSING");
        }

        // Check ffmpeg
        if (missingDeps.Contains("ffmpeg"))
        {
            message.AppendLine("❌ ffmpeg (system) - NOT FOUND\n");
            message.AppendLine("FFmpeg is required for audio conversion.");

            if (OperatingSystem.IsLinux())
            {
                message.AppendLine("\nInstallation instructions:");
                message.AppendLine("  Arch/Manjaro:    sudo pacman -S ffmpeg");
                message.AppendLine("  Ubuntu/Debian:   sudo apt install ffmpeg");
                message.AppendLine("  Fedora:          sudo dnf install ffmpeg");
                message.AppendLine("  Bazzite:         rpm-ostree install ffmpeg");
                message.AppendLine("                   (requires reboot)");
            }
            else if (OperatingSystem.IsWindows())
            {
                message.AppendLine("\nDownload from: https://ffmpeg.org/download.html");
            }
            else if (OperatingSystem.IsMacOS())
            {
                message.AppendLine("\nInstall with: brew install ffmpeg");
            }
        }
        else
        {
            message.AppendLine("✅ ffmpeg (system) - Found");
        }

        message.AppendLine("\n=== Disk Space Requirement ===\n");
        message.AppendLine("⚠️  Please ensure you have at least 20GB of free disk");
        message.AppendLine("   space available in your output directory.");

        message.AppendLine("\n" + (depsOk ? "All requirements satisfied!" : "Please install missing dependencies before proceeding."));

        if (_mainWindow != null)
        {
            Window? messageBox = null;
            messageBox = new Window
            {
                Title = depsOk ? "Requirements Check - OK" : "Requirements Check - Missing Dependencies",
                Width = 600,
                Height = 400,
                CanResize = false,
                WindowStartupLocation = WindowStartupLocation.CenterOwner,
                Content = new StackPanel
                {
                    Margin = new Thickness(20),
                    Spacing = 15,
                    Children =
                    {
                        new TextBox
                        {
                            Text = message.ToString(),
                            IsReadOnly = true,
                            TextWrapping = TextWrapping.NoWrap,
                            FontFamily = "Consolas,Courier New,monospace",
                            FontSize = 12,
                            AcceptsReturn = true,
                            Height = 280,
                            BorderThickness = new Thickness(0),
                            Background = Brushes.Transparent
                        },
                        new Button
                        {
                            Content = "OK",
                            Width = 100,
                            Height = 35,
                            HorizontalAlignment = HorizontalAlignment.Center,
                            Command = new RelayCommand(() => messageBox?.Close())
                        }
                    }
                }
            };

            await messageBox.ShowDialog(_mainWindow);
        }
    }

    private async Task<bool> ShowOutputDirectoryWarningAsync(int fileCount)
    {
        if (_mainWindow == null) return false;

        var result = false;
        Window? warningDialog = null;

        warningDialog = new Window
        {
            Title = "Output Directory Not Empty",
            Width = 550,
            Height = 300,
            CanResize = false,
            WindowStartupLocation = WindowStartupLocation.CenterOwner,
            Content = new StackPanel
            {
                Margin = new Thickness(20),
                Spacing = 15,
                Children =
                {
                    new TextBlock
                    {
                        Text = "⚠️ Warning: Output Directory Not Empty",
                        FontSize = 16,
                        FontWeight = Avalonia.Media.FontWeight.Bold
                    },
                    new TextBlock
                    {
                        Text = $"The output directory contains {fileCount} file(s) or folder(s).\n\n" +
                               "Installing TTW may overwrite existing files in this directory.\n\n" +
                               "It is recommended to use an empty directory for a fresh installation.\n\n" +
                               "Do you want to continue anyway?",
                        TextWrapping = TextWrapping.Wrap,
                        FontSize = 12
                    },
                    new StackPanel
                    {
                        Orientation = Avalonia.Layout.Orientation.Horizontal,
                        HorizontalAlignment = HorizontalAlignment.Center,
                        Spacing = 10,
                        Children =
                        {
                            new Button
                            {
                                Content = "Continue",
                                Width = 100,
                                Height = 35,
                                Command = new RelayCommand(() =>
                                {
                                    result = true;
                                    warningDialog?.Close();
                                })
                            },
                            new Button
                            {
                                Content = "Cancel",
                                Width = 100,
                                Height = 35,
                                Command = new RelayCommand(() =>
                                {
                                    result = false;
                                    warningDialog?.Close();
                                })
                            }
                        }
                    }
                }
            }
        };

        await warningDialog.ShowDialog(_mainWindow);
        return result;
    }

    private void LoadConfig()
    {
        var config = InstallConfig.FromFile("ttw-config.json");
        if (config != null)
        {
            Fallout3Path = config.Fallout3Root ?? "";
            FalloutNVPath = config.FalloutNVRoot ?? "";
            MpiPath = config.MpiPackagePath ?? "";
            OutputPath = config.DestinationPath ?? "";
            AppendLog("Loaded configuration from ttw-config.json");
        }
        else
        {
            // Create default config file if it doesn't exist
            var defaultConfig = new InstallConfig
            {
                Fallout3Root = "",
                FalloutNVRoot = "",
                MpiPackagePath = "",
                DestinationPath = ""
            };
            defaultConfig.SaveToFile("ttw-config.json");
            AppendLog("Created default configuration file: ttw-config.json");
        }
    }

    private void SaveConfig()
    {
        var config = new InstallConfig
        {
            Fallout3Root = Fallout3Path,
            FalloutNVRoot = FalloutNVPath,
            MpiPackagePath = MpiPath,
            DestinationPath = OutputPath
        };
        config.SaveToFile("ttw-config.json");
    }

    [RelayCommand]
    private async Task BrowseFallout3()
    {
        var path = await BrowseFolder("Select Fallout 3 Directory");
        if (!string.IsNullOrEmpty(path))
        {
            Fallout3Path = path;
            SaveConfig();
        }
    }

    [RelayCommand]
    private async Task BrowseFalloutNV()
    {
        var path = await BrowseFolder("Select Fallout New Vegas Directory");
        if (!string.IsNullOrEmpty(path))
        {
            FalloutNVPath = path;
            SaveConfig();
        }
    }

    [RelayCommand]
    private async Task BrowseMpi()
    {
        var path = await BrowseMpiFileOrFolder("Select TTW MPI File or Package Directory");
        if (!string.IsNullOrEmpty(path))
        {
            MpiPath = path;
            SaveConfig();
        }
    }

    [RelayCommand]
    private async Task BrowseOutput()
    {
        var path = await BrowseFolder("Select Output Directory");
        if (!string.IsNullOrEmpty(path))
        {
            OutputPath = path;
            SaveConfig();
        }
    }

    private async Task<string?> BrowseFolder(string title)
    {
        if (_mainWindow == null) return null;

        var folders = await _mainWindow.StorageProvider.OpenFolderPickerAsync(new FolderPickerOpenOptions
        {
            Title = title,
            AllowMultiple = false
        });

        return folders.Count > 0 ? folders[0].Path.LocalPath : null;
    }

    private async Task<string?> BrowseMpiFileOrFolder(string title)
    {
        if (_mainWindow == null) return null;

        // Try to open file picker first for .mpi files
        var files = await _mainWindow.StorageProvider.OpenFilePickerAsync(new FilePickerOpenOptions
        {
            Title = title,
            AllowMultiple = false,
            FileTypeFilter = new[]
            {
                new FilePickerFileType("MPI Package")
                {
                    Patterns = new[] { "*.mpi" }
                },
                new FilePickerFileType("All Files")
                {
                    Patterns = new[] { "*" }
                }
            }
        });

        if (files.Count > 0)
        {
            return files[0].Path.LocalPath;
        }

        return null;
    }

    [RelayCommand]
    private async Task StartInstall()
    {
        if (IsInstalling) return;

        // Validate paths
        if (string.IsNullOrWhiteSpace(Fallout3Path) ||
            string.IsNullOrWhiteSpace(FalloutNVPath) ||
            string.IsNullOrWhiteSpace(MpiPath) ||
            string.IsNullOrWhiteSpace(OutputPath))
        {
            AppendLog("❌ Error: All paths must be specified!");
            return;
        }

        // Check if output directory exists and is not empty
        if (Directory.Exists(OutputPath))
        {
            var filesInOutput = Directory.GetFileSystemEntries(OutputPath);
            if (filesInOutput.Length > 0)
            {
                // Show warning dialog
                if (_mainWindow != null)
                {
                    var result = await ShowOutputDirectoryWarningAsync(filesInOutput.Length);
                    if (!result)
                    {
                        AppendLog("Installation cancelled by user.");
                        return;
                    }
                }
            }
        }

        IsInstalling = true;
        InstallButtonText = "Installing...";
        _logBuilder.Clear();
        LogOutput = "";
        ProgressPercent = 0;
        ProgressText = "Starting...";

        SaveConfig();

        try
        {
            await Task.Run(() => RunInstallation());
            AppendLog("\n✅ Installation completed successfully!");
            ProgressPercent = 100;
            ProgressText = "Complete!";
        }
        catch (Exception ex)
        {
            AppendLog($"\n❌ Installation failed: {ex.Message}");
            ProgressText = "Failed";

            // Write error log to file for user to share
            try
            {
                var logPath = Path.Combine(Environment.CurrentDirectory, "ttw-installer-error.log");
                var errorLog = $"TTW Installer Error Log - {DateTime.Now}\n\n" +
                              $"Error: {ex.Message}\n\n" +
                              $"Stack Trace:\n{ex.StackTrace}\n\n" +
                              $"Full Log:\n{_logBuilder}";
                File.WriteAllText(logPath, errorLog);
                AppendLog($"\n📝 Error log saved to: {logPath}");
            }
            catch { }
        }
        finally
        {
            IsInstalling = false;
            InstallButtonText = "Start Installation";
        }
    }

    private void RunInstallation()
    {
        var config = new InstallConfig
        {
            Fallout3Root = Fallout3Path,
            FalloutNVRoot = FalloutNVPath,
            MpiPackagePath = MpiPath,
            DestinationPath = OutputPath,
            StartInstallation = true
        };

        AppendLog("=== TTW Installer ===\n");

        // Check system dependencies first
        AppendLog("=== Pre-Installation Checks ===\n");
        AppendLog("  Checking xdelta3 (bundled)...");
        AppendLog("  Checking ffmpeg (system)...");

        var (depsOk, missingDeps) = DependencyChecker.CheckDependencies();
        if (!depsOk)
        {
            AppendLog($"\n❌ Pre-installation checks failed!\n");
            AppendLog($"Missing: {string.Join(", ", missingDeps)}\n");

            // Show platform-specific install instructions for system dependencies only
            var systemDeps = missingDeps.Where(d => !d.Contains("bundled")).ToList();
            if (systemDeps.Count > 0 && OperatingSystem.IsLinux())
            {
                AppendLog("Installation instructions:\n");
                AppendLog($"  sudo pacman -S {string.Join(" ", systemDeps)}  # Arch/Manjaro\n");
                AppendLog($"  sudo apt install {string.Join(" ", systemDeps)}  # Ubuntu/Debian\n");
                AppendLog($"  sudo dnf install {string.Join(" ", systemDeps)}  # Fedora\n");
                AppendLog($"  rpm-ostree install {string.Join(" ", systemDeps)}  # Bazzite (requires reboot)\n");
            }

            throw new Exception($"Pre-installation checks failed: {string.Join(", ", missingDeps)}");
        }
        AppendLog("✅ All pre-installation checks passed\n");

        AppendLog("Validating configuration...");

        try
        {
            config.Validate();
            AppendLog("✅ Configuration valid\n");
        }
        catch (Exception ex)
        {
            throw new Exception($"Configuration error: {ex.Message}");
        }

        // Check if MPI extraction is needed
        string? extractedMpiPath = null;
        if (MpiExtractor.IsMpiFile(config.MpiPackagePath))
        {
            AppendLog("Detected .mpi file - extracting...");
            UpdateProgress(5, "Extracting MPI package...");
            try
            {
                extractedMpiPath = MpiExtractor.ExtractMpiToTemp(config.MpiPackagePath);
                config.MpiPackagePath = extractedMpiPath; // Update config to use extracted path
                AppendLog("✅ MPI extraction complete\n");
            }
            catch (Exception ex)
            {
                throw new Exception($"MPI extraction failed: {ex.Message}");
            }
        }

        try
        {
            RunInstallationInternal(config);
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

    private void RunInstallationInternal(InstallConfig config)
    {
        UpdateProgress(10, "Loading manifest...");
        var loader = new ManifestLoader();
        var manifestPath = Path.Combine(config.MpiPackagePath, "_package", "index.json");
        var manifest = loader.LoadFromFile(manifestPath);

        var locations = loader.GetLocations(manifest, 1);
        var assets = loader.ParseAssets(manifest);
        var checks = manifest.Checks;

        AppendLog($"Loaded {assets.Count} total assets");
        AppendLog($"Loaded {checks?.Count ?? 0} validation checks\n");

        UpdateProgress(10, "Running validation...");
        var locationResolver = new LocationResolver(locations, config);
        var validationService = new ValidationService(config, locationResolver);

        var (validationSuccess, validationErrors) = validationService.RunValidationChecksWithDetails(checks);
        if (!validationSuccess)
        {
            AppendLog("\n❌ Validation Failed:\n");
            AppendLog(validationErrors);
            throw new Exception($"Validation failed:\n\n{validationErrors}");
        }

        AppendLog("✅ Validation passed\n");

        // Create services
        var logger = new InstallationLogger();
        using var bsaReader = new BsaReader();
        using var bsaWriter = new BsaWriter(locations, config.DestinationPath);

        var assetProcessor = new AssetProcessor(locationResolver, bsaReader, config, bsaWriter, logger);

        // Filter assets
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

        // Create destination
        Directory.CreateDirectory(config.DestinationPath);

        // Process New assets (10-20%) - Parallelized
        UpdateProgress(15, $"Processing {newAssets.Count} new assets...");
        int newProcessed = 0;
        var parallelOptions = new ParallelOptions { MaxDegreeOfParallelism = 4 };
        Parallel.ForEach(newAssets, parallelOptions, asset =>
        {
            assetProcessor.ProcessAsset(asset);
            int current = Interlocked.Increment(ref newProcessed);
            Interlocked.Increment(ref processedAssets);
            if (current % 100 == 0)
            {
                double percent = 15 + (5.0 * current / newAssets.Count);
                UpdateProgress(percent, $"New assets: {current}/{newAssets.Count}");
            }
        });

        // Process Copy assets (20-50%) - Parallelized
        UpdateProgress(20, $"Processing {copyAssets.Count} copy operations...");
        int copyCount = 0;
        Parallel.ForEach(copyAssets, parallelOptions, asset =>
        {
            assetProcessor.ProcessAsset(asset);
            int current = Interlocked.Increment(ref copyCount);
            Interlocked.Increment(ref processedAssets);
            if (current % 500 == 0)
            {
                double percent = 20 + (30.0 * current / copyAssets.Count);
                UpdateProgress(percent, $"Copy: {current}/{copyAssets.Count}");
            }
        });

        // Process Patch assets (50-60%) - Parallelized
        UpdateProgress(50, $"Processing {patchAssets.Count} patches...");
        int patchCount = 0;
        Parallel.ForEach(patchAssets, parallelOptions, asset =>
        {
            assetProcessor.ProcessAsset(asset);
            int current = Interlocked.Increment(ref patchCount);
            Interlocked.Increment(ref processedAssets);
            if (current % 100 == 0)
            {
                double percent = 50 + (10.0 * current / patchAssets.Count);
                UpdateProgress(percent, $"Patches: {current}/{patchAssets.Count}");
            }
        });

        // Detect CPU cores for parallel audio processing
        var cpuCores = Environment.ProcessorCount;
        var parallelThreads = cpuCores; // Use ALL cores for ffmpeg - 100% CPU!
        AppendLog($"\nDetected {cpuCores} CPU cores, using {parallelThreads} threads for parallel audio processing (100% CPU)\n");

        // Process OggEnc2 assets (60-75%) - PARALLEL
        UpdateProgress(60, $"Processing {oggEnc2Assets.Count} OggEnc2 operations...");
        int oggCount = 0;
        object oggLock = new object();

        Parallel.ForEach(oggEnc2Assets,
            new ParallelOptions { MaxDegreeOfParallelism = parallelThreads },
            asset =>
            {
                assetProcessor.ProcessAsset(asset);

                lock (oggLock)
                {
                    oggCount++;
                    processedAssets++;
                    if (oggCount % 1000 == 0)
                    {
                        double percent = 60 + (15.0 * oggCount / oggEnc2Assets.Count);
                        UpdateProgress(percent, $"OggEnc2: {oggCount}/{oggEnc2Assets.Count}");
                    }
                }
            });

        // Process AudioEnc assets (75-80%) - PARALLEL
        UpdateProgress(75, $"Processing {audioEncAssets.Count} AudioEnc operations...");
        int audioCount = 0;
        object audioLock = new object();

        Parallel.ForEach(audioEncAssets,
            new ParallelOptions { MaxDegreeOfParallelism = parallelThreads },
            asset =>
            {
                assetProcessor.ProcessAsset(asset);

                lock (audioLock)
                {
                    audioCount++;
                    processedAssets++;
                    if (audioCount % 100 == 0)
                    {
                        double percent = 75 + (5.0 * audioCount / audioEncAssets.Count);
                        UpdateProgress(percent, $"AudioEnc: {audioCount}/{audioEncAssets.Count}");
                    }
                }
            });

        // Write BSAs (80-95%)
        UpdateProgress(80, "Writing BSA archives...");
        AppendLog("\n=== Writing BSA Archives ===");
        int failCount = bsaWriter.WriteAllBsas();

        if (failCount > 0)
        {
            AppendLog($"⚠️  {failCount} BSA(s) failed to write");
        }

        // Run post-commands (95-100%)
        UpdateProgress(95, "Running post-installation commands...");
        AppendLog("\n=== Running Post-Installation Commands ===");

        var postCommandRunner = new PostCommandRunner(config.DestinationPath, config);
        var postCommands = manifest.PostCommands ?? new List<PostCommand>();
        postCommandRunner.RunPostCommands(postCommands);

        UpdateProgress(100, "Installation complete!");
        AppendLog($"\n✅ Assets processed: {processedAssets}/{totalAssets}");

        // Display issues summary
        if (logger.HasIssues)
        {
            AppendLog($"\n{logger.GetSummary()}");
            AppendLog($"⚠️  Installation completed with {logger.ErrorCount} error(s), {logger.WarningCount} warning(s), {logger.MissingFileCount} missing file(s)\n");
        }
        else
        {
            AppendLog("\n✅ Installation completed successfully with no issues!\n");
        }

        // Write detailed log file
        logger.WriteLogFile(config.DestinationPath);
        AppendLog($"📝 Detailed installation log saved to: {Path.Combine(config.DestinationPath, "ttw-installation.log")}");
    }

    private void UpdateProgress(double percent, string text)
    {
        Dispatcher.UIThread.Post(() =>
        {
            ProgressPercent = percent;
            ProgressText = $"{percent:F1}% - {text}";
        });
    }

    private void AppendLog(string message)
    {
        _logBuilder.AppendLine(message);
        var logText = _logBuilder.ToString();
        Dispatcher.UIThread.Post(() =>
        {
            LogOutput = logText;
        });
    }

    // BSA Decompressor Methods
    private void AppendBsaLog(string message)
    {
        _bsaLogBuilder.AppendLine(message);
        var logText = _bsaLogBuilder.ToString();
        Dispatcher.UIThread.Post(() =>
        {
            BsaLogOutput = logText;
        });
    }

    private void UpdateBsaProgress(double percent, string text)
    {
        Dispatcher.UIThread.Post(() =>
        {
            BsaProgressPercent = percent;
            BsaProgressText = $"{percent:F1}% - {text}";
        });
    }

    [RelayCommand]
    private async Task BsaBrowseFalloutNV()
    {
        var path = await BrowseFolder("Select Fallout New Vegas Data Directory");
        if (!string.IsNullOrEmpty(path))
        {
            BsaFalloutNVPath = path;
            SaveBsaConfig();
        }
    }

    [RelayCommand]
    private async Task BsaBrowseMpi()
    {
        var path = await BrowseMpiFileOrFolder("Select FNV BSA Decompressor MPI File");
        if (!string.IsNullOrEmpty(path))
        {
            BsaMpiPath = path;
            SaveBsaConfig();
        }
    }

    [RelayCommand]
    private async Task BsaBrowseOutput()
    {
        var path = await BrowseFolder("Select Output Directory (for testing)");
        if (!string.IsNullOrEmpty(path))
        {
            BsaOutputPath = path;
            SaveBsaConfig();
        }
    }

    private void LoadBsaConfig()
    {
        var configPath = "bsa-decompressor-config.json";
        if (File.Exists(configPath))
        {
            try
            {
                var json = File.ReadAllText(configPath);
                var config = System.Text.Json.JsonSerializer.Deserialize<BsaDecompressorConfig>(json);
                if (config != null)
                {
                    BsaFalloutNVPath = config.FalloutNVDataPath ?? "";
                    BsaMpiPath = config.MpiPath ?? "";
                    BsaOutputPath = config.OutputPath ?? "";
                    AppendBsaLog("Loaded configuration from bsa-decompressor-config.json");
                }
            }
            catch
            {
                // Ignore errors loading config
            }
        }
    }

    private void SaveBsaConfig()
    {
        var config = new BsaDecompressorConfig
        {
            FalloutNVDataPath = BsaFalloutNVPath,
            MpiPath = BsaMpiPath,
            OutputPath = BsaOutputPath
        };

        try
        {
            var json = System.Text.Json.JsonSerializer.Serialize(config, new System.Text.Json.JsonSerializerOptions { WriteIndented = true });
            File.WriteAllText("bsa-decompressor-config.json", json);
        }
        catch
        {
            // Ignore save errors
        }
    }

    [RelayCommand]
    private async Task StartBsaDecompress()
    {
        if (IsInstalling) return;

        // Validate paths
        if (string.IsNullOrWhiteSpace(BsaFalloutNVPath) || string.IsNullOrWhiteSpace(BsaMpiPath))
        {
            AppendBsaLog("❌ Error: FNV path and MPI file must be specified!");
            return;
        }

        IsInstalling = true;
        BsaInstallButtonText = "Decompressing...";
        _bsaLogBuilder.Clear();
        BsaLogOutput = "";
        BsaProgressPercent = 0;
        BsaProgressText = "Starting...";

        SaveBsaConfig();

        try
        {
            await Task.Run(() => RunBsaDecompression());
            AppendBsaLog("\n✅ BSA Decompression completed successfully!");
            BsaProgressPercent = 100;
            BsaProgressText = "Complete!";
        }
        catch (Exception ex)
        {
            AppendBsaLog($"\n❌ BSA Decompression failed: {ex.Message}");
            BsaProgressText = "Failed";

            // Write error log
            try
            {
                var logPath = Path.Combine(Environment.CurrentDirectory, "bsa-decompressor-error.log");
                var errorLog = $"BSA Decompressor Error Log - {DateTime.Now}\n\n" +
                              $"Error: {ex.Message}\n\n" +
                              $"Stack Trace:\n{ex.StackTrace}\n\n" +
                              $"Full Log:\n{_bsaLogBuilder}";
                File.WriteAllText(logPath, errorLog);
                AppendBsaLog($"\n📝 Error log saved to: {logPath}");
            }
            catch { }
        }
        finally
        {
            IsInstalling = false;
            BsaInstallButtonText = "Start Decompression";
        }
    }

    private void RunBsaDecompression()
    {
        AppendBsaLog("=== FNV BSA Decompressor ===\n");
        AppendBsaLog($"FNV Data Path: {BsaFalloutNVPath}");
        AppendBsaLog($"MPI File: {BsaMpiPath}");

        // Determine output directory
        string outputPath = string.IsNullOrWhiteSpace(BsaOutputPath) ? BsaFalloutNVPath : BsaOutputPath;
        AppendBsaLog($"Output Path: {outputPath}\n");

        // Validate FNV path exists
        if (!Directory.Exists(BsaFalloutNVPath))
        {
            throw new Exception($"FNV Data directory not found: {BsaFalloutNVPath}");
        }

        // Check if MPI extraction is needed
        string? extractedMpiPath = null;
        if (MpiExtractor.IsMpiFile(BsaMpiPath))
        {
            AppendBsaLog("Detected .mpi file - extracting...");
            UpdateBsaProgress(5, "Extracting MPI package...");
            try
            {
                extractedMpiPath = MpiExtractor.ExtractMpiToTemp(BsaMpiPath);
                AppendBsaLog("✅ MPI extraction complete\n");
            }
            catch (Exception ex)
            {
                throw new Exception($"MPI extraction failed: {ex.Message}");
            }
        }
        else
        {
            extractedMpiPath = BsaMpiPath;
        }

        try
        {
            RunBsaDecompressionInternal(extractedMpiPath, outputPath);
        }
        finally
        {
            // Clean up extracted MPI if we created one
            if (MpiExtractor.IsMpiFile(BsaMpiPath) && extractedMpiPath != null)
            {
                MpiExtractor.CleanupTempDirectory(extractedMpiPath);
            }
        }
    }

    private void RunBsaDecompressionInternal(string packagePath, string outputPath)
    {
        UpdateBsaProgress(10, "Loading manifest...");
        var loader = new ManifestLoader();
        var manifestPath = Path.Combine(packagePath, "_package", "index.json");

        if (!File.Exists(manifestPath))
        {
            throw new Exception($"Manifest not found: {manifestPath}");
        }

        var manifest = loader.LoadFromFile(manifestPath);

        // Use profile 1 for decompression (has the NEW BSA locations)
        var locations = loader.GetLocations(manifest, 1);
        var assets = loader.ParseAssets(manifest);

        AppendBsaLog($"Loaded {assets.Count} assets");
        AppendBsaLog($"Loaded {locations.Count} locations\n");

        // Create a config-like structure for BSA decompression
        var config = new InstallConfig
        {
            FalloutNVRoot = BsaFalloutNVPath,
            DestinationPath = outputPath
        };

        // Create LocationResolver with modified locations to point to our paths
        var locationResolver = new LocationResolver(locations, config);

        // Create services
        using var bsaReader = new BsaReader();
        using var bsaWriter = new BsaWriter(locations, outputPath);
        var logger = new InstallationLogger();

        var assetProcessor = new AssetProcessor(locationResolver, bsaReader, config, bsaWriter, logger);

        // Process assets (should all be OpType 0 - Copy)
        UpdateBsaProgress(20, $"Processing {assets.Count} assets...");
        int processedCount = 0;
        int totalAssets = assets.Count;

        // Parallel processing with 4 threads for improved performance
        var parallelOptions = new ParallelOptions { MaxDegreeOfParallelism = 4 };
        Parallel.ForEach(assets, parallelOptions, asset =>
        {
            try
            {
                assetProcessor.ProcessAsset(asset);
                int current = Interlocked.Increment(ref processedCount);

                if (current % 1000 == 0)
                {
                    double percent = 20 + (60.0 * current / totalAssets);
                    UpdateBsaProgress(percent, $"Processed: {current}/{totalAssets}");
                }
            }
            catch (Exception ex)
            {
                AppendBsaLog($"⚠️  Failed to process {asset.SourcePath}: {ex.Message}");
            }
        });

        AppendBsaLog($"\n✅ Processed {processedCount}/{totalAssets} assets\n");

        // Write BSA archives (decompressed)
        UpdateBsaProgress(80, "Writing decompressed BSA archives...");
        AppendBsaLog("=== Writing Decompressed BSA Archives ===");
        int bsaFailCount = bsaWriter.WriteAllBsas();

        if (bsaFailCount > 0)
        {
            AppendBsaLog($"⚠️  {bsaFailCount} BSA(s) failed to write");
        }
        else
        {
            AppendBsaLog("✅ All BSA archives written successfully");
        }

        // Run post-commands (file renaming/deletion)
        UpdateBsaProgress(90, "Running post-installation commands...");
        AppendBsaLog("\n=== Running Post-Installation Commands ===");

        // IMPORTANT: Create a config for PostCommands where %FNVDATA% points to OUTPUT folder
        // This prevents PostCommands from deleting files in the source FNV folder!
        // Directly set FalloutNVData to outputPath (bypassing the computed property)
        var postCommandConfig = new InstallConfig
        {
            FalloutNVData = outputPath,  // Directly override - files are in outputPath, not outputPath/Data
            DestinationPath = outputPath
        };

        var postCommandRunner = new PostCommandRunner(outputPath, postCommandConfig);
        var postCommands = manifest.PostCommands ?? new List<PostCommand>();
        int postFailCount = postCommandRunner.RunPostCommands(postCommands);

        if (postFailCount > 0)
        {
            AppendBsaLog($"⚠️  {postFailCount} post-command(s) failed");
        }
        else
        {
            AppendBsaLog("✅ All post-commands executed successfully");
        }

        UpdateBsaProgress(100, "Decompression complete!");

        if (logger.HasIssues)
        {
            AppendBsaLog($"\n{logger.GetSummary()}");
            AppendBsaLog($"⚠️  Completed with {logger.ErrorCount} error(s), {logger.WarningCount} warning(s)\n");
        }
        else
        {
            AppendBsaLog("\n✅ Decompression completed successfully with no issues!\n");
        }

        // Write log file
        logger.WriteLogFile(outputPath);
        AppendBsaLog($"📝 Detailed log saved to: {Path.Combine(outputPath, "ttw-installation.log")}");
    }

    // BSA Decompressor Config Model
    private class BsaDecompressorConfig
    {
        public string? FalloutNVDataPath { get; set; }
        public string? MpiPath { get; set; }
        public string? OutputPath { get; set; }
    }

    // Oblivion BSA Decompressor Methods
    private void AppendOblivionLog(string message)
    {
        _oblivionLogBuilder.AppendLine(message);
        var logText = _oblivionLogBuilder.ToString();
        Dispatcher.UIThread.Post(() =>
        {
            OblivionLogOutput = logText;
        });
    }

    private void UpdateOblivionProgress(double percent, string text)
    {
        Dispatcher.UIThread.Post(() =>
        {
            OblivionProgressPercent = percent;
            OblivionProgressText = $"{percent:F1}% - {text}";
        });
    }

    [RelayCommand]
    private async Task OblivionBrowseData()
    {
        var path = await BrowseFolder("Select Oblivion Data Directory");
        if (!string.IsNullOrEmpty(path))
        {
            OblivionDataPath = path;
            SaveOblivionConfig();
        }
    }

    [RelayCommand]
    private async Task OblivionBrowseMpi()
    {
        var path = await BrowseMpiFileOrFolder("Select Oblivion BSA Decompressor MPI File");
        if (!string.IsNullOrEmpty(path))
        {
            OblivionMpiPath = path;
            SaveOblivionConfig();
        }
    }

    [RelayCommand]
    private async Task OblivionBrowseOutput()
    {
        var path = await BrowseFolder("Select Output Directory (for testing)");
        if (!string.IsNullOrEmpty(path))
        {
            OblivionOutputPath = path;
            SaveOblivionConfig();
        }
    }

    private void LoadOblivionConfig()
    {
        var configPath = "oblivion-decompressor-config.json";
        if (File.Exists(configPath))
        {
            try
            {
                var json = File.ReadAllText(configPath);
                var config = System.Text.Json.JsonSerializer.Deserialize<OblivionDecompressorConfig>(json);
                if (config != null)
                {
                    OblivionDataPath = config.OblivionDataPath ?? "";
                    OblivionMpiPath = config.MpiPath ?? "";
                    OblivionOutputPath = config.OutputPath ?? "";
                    AppendOblivionLog("Loaded configuration from oblivion-decompressor-config.json");
                }
            }
            catch
            {
                // Ignore errors loading config
            }
        }
    }

    private void SaveOblivionConfig()
    {
        var config = new OblivionDecompressorConfig
        {
            OblivionDataPath = OblivionDataPath,
            MpiPath = OblivionMpiPath,
            OutputPath = OblivionOutputPath
        };

        try
        {
            var json = System.Text.Json.JsonSerializer.Serialize(config, new System.Text.Json.JsonSerializerOptions { WriteIndented = true });
            File.WriteAllText("oblivion-decompressor-config.json", json);
        }
        catch
        {
            // Ignore save errors
        }
    }

    [RelayCommand]
    private async Task StartOblivionDecompress()
    {
        if (IsInstalling) return;

        // Validate paths
        if (string.IsNullOrWhiteSpace(OblivionDataPath) || string.IsNullOrWhiteSpace(OblivionMpiPath))
        {
            AppendOblivionLog("❌ Error: Oblivion path and MPI file must be specified!");
            return;
        }

        IsInstalling = true;
        OblivionInstallButtonText = "Decompressing...";
        _oblivionLogBuilder.Clear();
        OblivionLogOutput = "";
        OblivionProgressPercent = 0;
        OblivionProgressText = "Starting...";

        SaveOblivionConfig();

        try
        {
            await Task.Run(() => RunOblivionDecompression());
            AppendOblivionLog("\n✅ BSA Decompression completed successfully!");
            OblivionProgressPercent = 100;
            OblivionProgressText = "Complete!";
        }
        catch (Exception ex)
        {
            AppendOblivionLog($"\n❌ BSA Decompression failed: {ex.Message}");
            OblivionProgressText = "Failed";

            // Write error log
            try
            {
                var logPath = Path.Combine(Environment.CurrentDirectory, "oblivion-decompressor-error.log");
                var errorLog = $"Oblivion BSA Decompressor Error Log - {DateTime.Now}\n\n" +
                              $"Error: {ex.Message}\n\n" +
                              $"Stack Trace:\n{ex.StackTrace}\n\n" +
                              $"Full Log:\n{_oblivionLogBuilder}";
                File.WriteAllText(logPath, errorLog);
                AppendOblivionLog($"\n📝 Error log saved to: {logPath}");
            }
            catch { }
        }
        finally
        {
            IsInstalling = false;
            OblivionInstallButtonText = "Start Decompression";
        }
    }

    private void RunOblivionDecompression()
    {
        AppendOblivionLog("=== Oblivion BSA Decompressor ===\n");
        AppendOblivionLog($"Oblivion Data Path: {OblivionDataPath}");
        AppendOblivionLog($"MPI File: {OblivionMpiPath}");

        // Determine output directory
        string outputPath = string.IsNullOrWhiteSpace(OblivionOutputPath) ? OblivionDataPath : OblivionOutputPath;
        AppendOblivionLog($"Output Path: {outputPath}\n");

        // Validate Oblivion path exists
        if (!Directory.Exists(OblivionDataPath))
        {
            throw new Exception($"Oblivion Data directory not found: {OblivionDataPath}");
        }

        // Check if MPI extraction is needed
        string? extractedMpiPath = null;
        if (MpiExtractor.IsMpiFile(OblivionMpiPath))
        {
            AppendOblivionLog("Detected .mpi file - extracting...");
            UpdateOblivionProgress(5, "Extracting MPI package...");
            try
            {
                extractedMpiPath = MpiExtractor.ExtractMpiToTemp(OblivionMpiPath);
                AppendOblivionLog("✅ MPI extraction complete\n");
            }
            catch (Exception ex)
            {
                throw new Exception($"MPI extraction failed: {ex.Message}");
            }
        }
        else
        {
            extractedMpiPath = OblivionMpiPath;
        }

        try
        {
            RunOblivionDecompressionInternal(extractedMpiPath, outputPath);
        }
        finally
        {
            // Clean up extracted MPI if we created one
            if (MpiExtractor.IsMpiFile(OblivionMpiPath) && extractedMpiPath != null)
            {
                MpiExtractor.CleanupTempDirectory(extractedMpiPath);
            }
        }
    }

    private void RunOblivionDecompressionInternal(string packagePath, string outputPath)
    {
        UpdateOblivionProgress(10, "Loading manifest...");
        var loader = new ManifestLoader();
        var manifestPath = Path.Combine(packagePath, "_package", "index.json");

        if (!File.Exists(manifestPath))
        {
            throw new Exception($"Manifest not found: {manifestPath}");
        }

        var manifest = loader.LoadFromFile(manifestPath);

        // Use profile 1 for decompression (has the NEW BSA locations)
        var locations = loader.GetLocations(manifest, 1);
        var assets = loader.ParseAssets(manifest);

        AppendOblivionLog($"Loaded {assets.Count} assets");
        AppendOblivionLog($"Loaded {locations.Count} locations\n");

        // Create a config-like structure for BSA decompression
        var config = new InstallConfig
        {
            OblivionRoot = OblivionDataPath,
            DestinationPath = outputPath
        };

        // Create LocationResolver with modified locations to point to our paths
        var locationResolver = new LocationResolver(locations, config);

        // Create services
        using var bsaReader = new BsaReader();
        using var bsaWriter = new BsaWriter(locations, outputPath);
        var logger = new InstallationLogger();

        var assetProcessor = new AssetProcessor(locationResolver, bsaReader, config, bsaWriter, logger);

        // Process assets (should all be OpType 0 - Copy)
        UpdateOblivionProgress(20, $"Processing {assets.Count} assets...");
        int processedCount = 0;
        int totalAssets = assets.Count;

        // Parallel processing with 4 threads for improved performance
        var parallelOptions = new ParallelOptions { MaxDegreeOfParallelism = 4 };
        Parallel.ForEach(assets, parallelOptions, asset =>
        {
            try
            {
                assetProcessor.ProcessAsset(asset);
                int current = Interlocked.Increment(ref processedCount);

                if (current % 1000 == 0)
                {
                    double percent = 20 + (60.0 * current / totalAssets);
                    UpdateOblivionProgress(percent, $"Processed: {current}/{totalAssets}");
                }
            }
            catch (Exception ex)
            {
                AppendOblivionLog($"⚠️  Failed to process {asset.SourcePath}: {ex.Message}");
            }
        });

        AppendOblivionLog($"\n✅ Processed {processedCount}/{totalAssets} assets\n");

        // Write BSA archives (decompressed)
        UpdateOblivionProgress(80, "Writing decompressed BSA archives...");
        AppendOblivionLog("=== Writing Decompressed BSA Archives ===");
        int bsaFailCount = bsaWriter.WriteAllBsas();

        if (bsaFailCount > 0)
        {
            AppendOblivionLog($"⚠️  {bsaFailCount} BSA(s) failed to write");
        }
        else
        {
            AppendOblivionLog("✅ All BSA archives written successfully");
        }

        // Run post-commands (file renaming/deletion)
        UpdateOblivionProgress(90, "Running post-installation commands...");
        AppendOblivionLog("\n=== Running Post-Installation Commands ===");

        // IMPORTANT: Create a config for PostCommands where %TES4DATA% points to OUTPUT folder
        // This prevents PostCommands from deleting files in the source Oblivion folder!
        // Directly set OblivionData to outputPath (bypassing the computed property)
        var postCommandConfig = new InstallConfig
        {
            OblivionData = outputPath,  // Directly override - files are in outputPath, not outputPath/Data
            DestinationPath = outputPath
        };

        var postCommandRunner = new PostCommandRunner(outputPath, postCommandConfig);
        var postCommands = manifest.PostCommands ?? new List<PostCommand>();
        int postFailCount = postCommandRunner.RunPostCommands(postCommands);

        if (postFailCount > 0)
        {
            AppendOblivionLog($"⚠️  {postFailCount} post-command(s) failed");
        }
        else
        {
            AppendOblivionLog("✅ All post-commands executed successfully");
        }

        UpdateOblivionProgress(100, "Decompression complete!");

        if (logger.HasIssues)
        {
            AppendOblivionLog($"\n{logger.GetSummary()}");
            AppendOblivionLog($"⚠️  Completed with {logger.ErrorCount} error(s), {logger.WarningCount} warning(s)\n");
        }
        else
        {
            AppendOblivionLog("\n✅ Decompression completed successfully with no issues!\n");
        }

        // Write log file
        logger.WriteLogFile(outputPath);
        AppendOblivionLog($"📝 Detailed log saved to: {Path.Combine(outputPath, "ttw-installation.log")}");
    }

    // Oblivion BSA Decompressor Config Model
    private class OblivionDecompressorConfig
    {
        public string? OblivionDataPath { get; set; }
        public string? MpiPath { get; set; }
        public string? OutputPath { get; set; }
    }
}
