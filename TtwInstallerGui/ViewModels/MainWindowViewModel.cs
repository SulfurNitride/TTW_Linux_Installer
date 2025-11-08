using System;
using System.IO;
using System.Linq;
using System.Text;
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

    private StringBuilder _logBuilder = new StringBuilder();
    private Window? _mainWindow;

    public void SetMainWindow(Window window)
    {
        _mainWindow = window;
        LoadConfig();
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
            Fallout3Path = path;
    }

    [RelayCommand]
    private async Task BrowseFalloutNV()
    {
        var path = await BrowseFolder("Select Fallout New Vegas Directory");
        if (!string.IsNullOrEmpty(path))
            FalloutNVPath = path;
    }

    [RelayCommand]
    private async Task BrowseMpi()
    {
        var path = await BrowseMpiFileOrFolder("Select TTW MPI File or Package Directory");
        if (!string.IsNullOrEmpty(path))
            MpiPath = path;
    }

    [RelayCommand]
    private async Task BrowseOutput()
    {
        var path = await BrowseFolder("Select Output Directory");
        if (!string.IsNullOrEmpty(path))
            OutputPath = path;
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

        // Process New assets (10-20%)
        UpdateProgress(15, $"Processing {newAssets.Count} new assets...");
        foreach (var asset in newAssets)
        {
            assetProcessor.ProcessAsset(asset);
            processedAssets++;
            if (processedAssets % 100 == 0)
            {
                double percent = 15 + (5.0 * processedAssets / newAssets.Count);
                UpdateProgress(percent, $"New assets: {processedAssets}/{newAssets.Count}");
            }
        }

        // Process Copy assets (20-50%)
        UpdateProgress(20, $"Processing {copyAssets.Count} copy operations...");
        int copyCount = 0;
        foreach (var asset in copyAssets)
        {
            assetProcessor.ProcessAsset(asset);
            copyCount++;
            processedAssets++;
            if (copyCount % 500 == 0)
            {
                double percent = 20 + (30.0 * copyCount / copyAssets.Count);
                UpdateProgress(percent, $"Copy: {copyCount}/{copyAssets.Count}");
            }
        }

        // Process Patch assets (50-60%)
        UpdateProgress(50, $"Processing {patchAssets.Count} patches...");
        int patchCount = 0;
        foreach (var asset in patchAssets)
        {
            assetProcessor.ProcessAsset(asset);
            patchCount++;
            processedAssets++;
            if (patchCount % 100 == 0)
            {
                double percent = 50 + (10.0 * patchCount / patchAssets.Count);
                UpdateProgress(percent, $"Patches: {patchCount}/{patchAssets.Count}");
            }
        }

        // Detect CPU cores for parallel audio processing
        var cpuCores = Environment.ProcessorCount;
        var parallelThreads = Math.Max(1, cpuCores - 2); // Leave 2 cores for system
        AppendLog($"\nDetected {cpuCores} CPU cores, using {parallelThreads} threads for parallel audio processing\n");

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

        var postCommandRunner = new PostCommandRunner(config.DestinationPath);
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
}
