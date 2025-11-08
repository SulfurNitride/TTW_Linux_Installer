using System;
using Avalonia.Controls;
using TtwInstallerGui.ViewModels;

namespace TtwInstallerGui.Views;

public partial class MainWindow : Window
{
    public MainWindow()
    {
        InitializeComponent();

        // Subscribe to DataContext changes to set window reference
        DataContextChanged += OnDataContextChanged;
    }

    private void OnDataContextChanged(object? sender, EventArgs e)
    {
        // Pass window reference to ViewModel for folder browsing
        if (DataContext is MainWindowViewModel vm)
        {
            vm.SetMainWindow(this);
        }
    }
}