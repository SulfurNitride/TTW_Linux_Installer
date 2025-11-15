using System;
using System.Globalization;
using System.IO;
using Avalonia.Data.Converters;

namespace TtwInstallerGui.Views;

public class FileNameConverter : IValueConverter
{
    public static readonly FileNameConverter Instance = new();

    public object? Convert(object? value, Type targetType, object? parameter, CultureInfo culture)
    {
        if (value is string path)
        {
            return Path.GetFileName(path);
        }
        return value;
    }

    public object? ConvertBack(object? value, Type targetType, object? parameter, CultureInfo culture)
    {
        throw new NotImplementedException();
    }
}
