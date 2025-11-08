using TtwInstaller.Models;

namespace TtwInstaller.Services;

/// <summary>
/// Resolves location indices to actual file paths
/// </summary>
public class LocationResolver
{
    private readonly List<Location> _locations;
    private readonly InstallConfig _config;

    public LocationResolver(List<Location> locations, InstallConfig config)
    {
        _locations = locations;
        _config = config;
    }

    /// <summary>
    /// Resolve a location to its actual path
    /// </summary>
    public string ResolvePath(int locationIndex)
    {
        if (locationIndex < 0 || locationIndex >= _locations.Count)
        {
            throw new ArgumentOutOfRangeException(nameof(locationIndex),
                $"Location index {locationIndex} is out of range (0-{_locations.Count - 1})");
        }

        var location = _locations[locationIndex];
        return ResolveVariables(location.Value ?? string.Empty);
    }

    /// <summary>
    /// Get location by index
    /// </summary>
    public Location GetLocation(int locationIndex)
    {
        if (locationIndex < 0 || locationIndex >= _locations.Count)
        {
            throw new ArgumentOutOfRangeException(nameof(locationIndex));
        }

        return _locations[locationIndex];
    }

    /// <summary>
    /// Check if location is a BSA file
    /// </summary>
    public bool IsBsaLocation(int locationIndex)
    {
        var location = GetLocation(locationIndex);
        return location.Type == 1; // Type 1 = BSA source
    }

    /// <summary>
    /// Check if location is a BSA creation target
    /// </summary>
    public bool IsBsaCreationLocation(int locationIndex)
    {
        var location = GetLocation(locationIndex);
        return location.Type == 2; // Type 2 = BSA creation
    }

    /// <summary>
    /// Resolve variables in a path
    /// </summary>
    private string ResolveVariables(string path)
    {
        var resolved = path;

        // Replace path variables
        resolved = resolved.Replace("%FO3ROOT%", _config.Fallout3Root);
        resolved = resolved.Replace("%FO3DATA%", _config.Fallout3Data);
        resolved = resolved.Replace("%FNVROOT%", _config.FalloutNVRoot);
        resolved = resolved.Replace("%FNVDATA%", _config.FalloutNVData);
        resolved = resolved.Replace("%DESTINATION%", _config.DestinationPath);

        // Convert Windows paths to Unix paths if needed
        if (Path.DirectorySeparatorChar == '/')
        {
            resolved = resolved.Replace('\\', '/');
        }

        return resolved;
    }

    /// <summary>
    /// Get BSA path for a location
    /// </summary>
    public string GetBsaPath(int locationIndex)
    {
        if (!IsBsaLocation(locationIndex))
        {
            throw new InvalidOperationException($"Location {locationIndex} is not a BSA location");
        }

        return ResolvePath(locationIndex);
    }

    /// <summary>
    /// Get directory path for a location
    /// </summary>
    public string GetDirectoryPath(int locationIndex)
    {
        var location = GetLocation(locationIndex);

        if (location.Type == 0) // Directory location
        {
            return ResolvePath(locationIndex);
        }
        else if (location.Type == 2) // BSA creation target
        {
            // Return the directory containing the BSA
            var bsaPath = ResolvePath(locationIndex);
            return Path.GetDirectoryName(bsaPath) ?? string.Empty;
        }

        throw new InvalidOperationException($"Cannot get directory path for location type {location.Type}");
    }
}
