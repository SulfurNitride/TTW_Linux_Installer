using System.Text.Json;
using TtwInstaller.Models;

namespace TtwInstaller.Services;

/// <summary>
/// Loads and parses TTW installation manifest
/// </summary>
public class ManifestLoader
{
    private readonly JsonSerializerOptions _jsonOptions;

    public ManifestLoader()
    {
        _jsonOptions = new JsonSerializerOptions
        {
            PropertyNameCaseInsensitive = true,
            AllowTrailingCommas = true,
            ReadCommentHandling = JsonCommentHandling.Skip
        };
    }

    /// <summary>
    /// Load manifest from JSON file
    /// </summary>
    public TtwManifest LoadFromFile(string manifestPath)
    {
        if (!File.Exists(manifestPath))
        {
            throw new FileNotFoundException($"Manifest not found: {manifestPath}");
        }

        Console.WriteLine($"Loading manifest from: {manifestPath}");

        var json = File.ReadAllText(manifestPath);
        var manifest = JsonSerializer.Deserialize<TtwManifest>(json, _jsonOptions);

        if (manifest == null)
        {
            throw new InvalidOperationException("Failed to deserialize manifest");
        }

        Console.WriteLine($"  Package: {manifest.Package?.Title} v{manifest.Package?.Version}");
        Console.WriteLine($"  Variables: {manifest.Variables?[0]?.Count ?? 0} defined");
        Console.WriteLine($"  Locations: {manifest.Locations?[0]?.Count ?? 0} defined");
        Console.WriteLine($"  Assets: {manifest.Assets?.Count ?? 0} operations");

        return manifest;
    }

    /// <summary>
    /// Parse assets from manifest
    /// </summary>
    public List<Asset> ParseAssets(TtwManifest manifest)
    {
        if (manifest.Assets == null || manifest.Assets.Count == 0)
        {
            return new List<Asset>();
        }

        var assetArrays = manifest.Assets;
        var assets = new List<Asset>(assetArrays.Count);

        Console.WriteLine($"\nParsing {assetArrays.Count} assets...");

        int successCount = 0;
        int failCount = 0;

        foreach (var assetObj in assetArrays)
        {
            try
            {
                if (assetObj is JsonElement element)
                {
                    if (element.ValueKind == JsonValueKind.Array)
                    {
                        var asset = ParseAssetFromJsonElement(element);
                        assets.Add(asset);
                        successCount++;
                    }
                    else
                    {
                        failCount++;
                        if (failCount <= 3)
                        {
                            Console.WriteLine($"Warning: JsonElement is not an array, it's: {element.ValueKind}");
                            if (failCount == 1)
                            {
                                Console.WriteLine($"Raw value: {element.GetRawText().Substring(0, Math.Min(200, element.GetRawText().Length))}");
                            }
                        }
                    }
                }
                else
                {
                    failCount++;
                    if (failCount <= 3)
                    {
                        Console.WriteLine($"Warning: Asset object is not a JsonElement: {assetObj?.GetType().Name}");
                    }
                }
            }
            catch (Exception ex)
            {
                failCount++;
                if (failCount <= 3)
                {
                    Console.WriteLine($"Warning: Failed to parse asset: {ex.Message}");
                }
            }
        }

        if (failCount > 3)
        {
            Console.WriteLine($"Warning: {failCount - 3} more assets failed to parse (messages suppressed)");
        }

        Console.WriteLine($"Successfully parsed {assets.Count} assets");

        // Print operation type summary
        var summary = assets.GroupBy(a => a.OpType)
            .OrderBy(g => g.Key)
            .Select(g => $"  OpType {g.Key}: {g.Count()} operations");

        Console.WriteLine("\nOperation summary:");
        foreach (var line in summary)
        {
            Console.WriteLine(line);
        }

        return assets;
    }

    /// <summary>
    /// Get locations for a specific profile
    /// </summary>
    public List<Location> GetLocations(TtwManifest manifest, int profileIndex = 0)
    {
        if (manifest.Locations == null || manifest.Locations.Count == 0)
        {
            return new List<Location>();
        }

        if (profileIndex >= manifest.Locations.Count)
        {
            throw new ArgumentException($"Invalid profile index: {profileIndex}");
        }

        return manifest.Locations[profileIndex];
    }

    /// <summary>
    /// Get variables for a specific profile
    /// </summary>
    public List<Variable> GetVariables(TtwManifest manifest, int profileIndex = 0)
    {
        if (manifest.Variables == null || manifest.Variables.Count == 0)
        {
            return new List<Variable>();
        }

        if (profileIndex >= manifest.Variables.Count)
        {
            throw new ArgumentException($"Invalid profile index: {profileIndex}");
        }

        return manifest.Variables[profileIndex];
    }

    private Asset ParseAssetFromJsonElement(JsonElement element)
    {
        if (element.ValueKind != JsonValueKind.Array)
        {
            throw new ArgumentException("Asset must be a JSON array");
        }

        var arrayLength = element.GetArrayLength();
        if (arrayLength < 7)
        {
            throw new ArgumentException($"Invalid asset array length: {arrayLength}");
        }

        var enumerator = element.EnumerateArray();
        var items = enumerator.ToArray();

        var asset = new Asset
        {
            Tags = items[0].GetInt32(),
            OpType = items[1].GetInt32(),
            Params = items[2].GetString() ?? string.Empty,
            Status = items[3].GetInt32(),
            SourceLoc = items[4].GetInt32(),
            TargetLoc = items[5].GetInt32(),
            SourcePath = items[6].GetString() ?? string.Empty
        };

        // TargetPath is optional
        if (arrayLength > 7)
        {
            asset.TargetPath = items[7].GetString();
        }

        return asset;
    }

    private int GetInt(object obj)
    {
        if (obj is JsonElement element)
        {
            return element.GetInt32();
        }
        return Convert.ToInt32(obj);
    }

    private string GetString(object obj)
    {
        if (obj is JsonElement element)
        {
            return element.GetString() ?? string.Empty;
        }
        return obj?.ToString() ?? string.Empty;
    }
}
