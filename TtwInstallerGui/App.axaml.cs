using System.IO;
using System.Linq;
using Avalonia;
using Avalonia.Controls.ApplicationLifetimes;
using Avalonia.Data.Core;
using Avalonia.Data.Core.Plugins;
using Avalonia.Markup.Xaml;
using TtwInstallerGui.ViewModels;
using TtwInstallerGui.Views;
using TtwInstaller.Services;

namespace TtwInstallerGui;

public partial class App : Application
{
    public override void Initialize()
    {
        AvaloniaXamlLoader.Load(this);

        // Clean up any stale temp directories from previous runs
        CleanupStaleTempDirectories();
    }

    public override void OnFrameworkInitializationCompleted()
    {
        if (ApplicationLifetime is IClassicDesktopStyleApplicationLifetime desktop)
        {
            // Avoid duplicate validations from both Avalonia and the CommunityToolkit.
            // More info: https://docs.avaloniaui.net/docs/guides/development-guides/data-validation#manage-validationplugins
            DisableAvaloniaDataAnnotationValidation();
            desktop.MainWindow = new MainWindow
            {
                DataContext = new MainWindowViewModel(),
            };

            // Register cleanup on app exit
            desktop.ShutdownRequested += OnShutdownRequested;
        }

        base.OnFrameworkInitializationCompleted();
    }

    private void DisableAvaloniaDataAnnotationValidation()
    {
        // Get an array of plugins to remove
        var dataValidationPluginsToRemove =
            BindingPlugins.DataValidators.OfType<DataAnnotationsValidationPlugin>().ToArray();

        // remove each entry found
        foreach (var plugin in dataValidationPluginsToRemove)
        {
            BindingPlugins.DataValidators.Remove(plugin);
        }
    }

    private void OnShutdownRequested(object? sender, ShutdownRequestedEventArgs e)
    {
        // Clean up temp directories on exit
        TempDirectoryTracker.CleanupAll();
    }

    private void CleanupStaleTempDirectories()
    {
        // Clean up any leftover temp directories from previous runs/crashes
        try
        {
            var tempPath = Path.GetTempPath();
            var staleDirs = Directory.GetDirectories(tempPath, "ttw_mpi_*");

            foreach (var dir in staleDirs)
            {
                try
                {
                    Directory.Delete(dir, true);
                }
                catch
                {
                    // Ignore errors - directory might be in use by another instance
                }
            }
        }
        catch
        {
            // Ignore errors in cleanup
        }
    }
}