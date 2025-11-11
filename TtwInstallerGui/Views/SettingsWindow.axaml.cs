using Avalonia.Controls;
using Avalonia.Interactivity;
using TtwInstallerGui.ViewModels;

namespace TtwInstallerGui.Views;

public partial class SettingsWindow : Window
{
    public SettingsWindow()
    {
        InitializeComponent();
    }

    private void CloseButton_Click(object? sender, RoutedEventArgs e)
    {
        Close();
    }
}
