using System;
using System.IO;
using System.Text.Json;

namespace TtwInstallerGui.Models;

/// <summary>
/// User configuration for the Universal MPI Installer GUI
/// Stores game paths and preferences to persist between sessions
/// </summary>
public class UserConfig
{
    public string Fallout3Path { get; set; } = "";
    public string FalloutNVPath { get; set; } = "";
    public string OblivionPath { get; set; } = "";
    public string OutputPath { get; set; } = "";
    public string LastMpiPath { get; set; } = "";
    public bool CreateBsas { get; set; } = true; // Default: ON

    /// <summary>
    /// Get the default config file path (next to the executable)
    /// </summary>
    public static string GetConfigPath()
    {
        string exeDir = AppDomain.CurrentDomain.BaseDirectory;
        return Path.Combine(exeDir, "gui-config.json");
    }

    /// <summary>
    /// Load user config from disk, or create default if not exists
    /// </summary>
    public static UserConfig Load()
    {
        string configPath = GetConfigPath();

        if (!File.Exists(configPath))
        {
            return new UserConfig();
        }

        try
        {
            string json = File.ReadAllText(configPath);
            var config = JsonSerializer.Deserialize<UserConfig>(json);
            return config ?? new UserConfig();
        }
        catch
        {
            // If config is corrupted, return default
            return new UserConfig();
        }
    }

    /// <summary>
    /// Save user config to disk
    /// </summary>
    public void Save()
    {
        try
        {
            string configPath = GetConfigPath();
            var options = new JsonSerializerOptions
            {
                WriteIndented = true
            };
            string json = JsonSerializer.Serialize(this, options);
            File.WriteAllText(configPath, json);
        }
        catch
        {
            // Silently fail if we can't save config
        }
    }
}
