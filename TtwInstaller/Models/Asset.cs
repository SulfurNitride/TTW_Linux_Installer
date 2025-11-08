namespace TtwInstaller.Models;

/// <summary>
/// Represents an installation asset operation
/// Asset format: [Tags, OpType, Params, Status, SourceLoc, TargetLoc, SourcePath, TargetPath]
/// </summary>
public class Asset
{
    /// <summary>
    /// Tag bitmask (FO3=1, FNV=2, TTW=512, etc.)
    /// </summary>
    public int Tags { get; set; }

    /// <summary>
    /// Operation type: 0=Copy, 1=New, 2=Patch, 4=OggEnc2, 5=AudioEnc
    /// </summary>
    public int OpType { get; set; }

    /// <summary>
    /// Operation parameters (e.g., "-f:24000 -q:5" for audio encoding)
    /// </summary>
    public string Params { get; set; } = string.Empty;

    /// <summary>
    /// Status flags
    /// </summary>
    public int Status { get; set; }

    /// <summary>
    /// Source location index (into Locations array)
    /// </summary>
    public int SourceLoc { get; set; }

    /// <summary>
    /// Target location index (into Locations array)
    /// </summary>
    public int TargetLoc { get; set; }

    /// <summary>
    /// Source file path (relative to source location)
    /// </summary>
    public string SourcePath { get; set; } = string.Empty;

    /// <summary>
    /// Target file path (relative to target location)
    /// </summary>
    public string? TargetPath { get; set; }

    /// <summary>
    /// Parse asset from JSON array
    /// </summary>
    public static Asset FromJsonArray(List<object> array)
    {
        var asset = new Asset
        {
            Tags = Convert.ToInt32(array[0]),
            OpType = Convert.ToInt32(array[1]),
            Params = array[2]?.ToString() ?? string.Empty,
            Status = Convert.ToInt32(array[3]),
            SourceLoc = Convert.ToInt32(array[4]),
            TargetLoc = Convert.ToInt32(array[5]),
            SourcePath = array[6]?.ToString() ?? string.Empty
        };

        // TargetPath is optional (some assets only have 7 elements)
        if (array.Count > 7)
        {
            asset.TargetPath = array[7]?.ToString();
        }

        return asset;
    }

    public string GetEffectiveTargetPath()
    {
        return TargetPath ?? SourcePath;
    }

    public override string ToString()
    {
        return $"[Op{OpType}] {SourcePath} -> {GetEffectiveTargetPath()}";
    }
}

/// <summary>
/// Asset operation types
/// </summary>
public enum AssetOpType
{
    /// <summary>Copy file from source BSA/location</summary>
    Copy = 0,

    /// <summary>New file from MPI package</summary>
    New = 1,

    /// <summary>Apply binary patch (.xd3)</summary>
    Patch = 2,

    /// <summary>OggEnc2 audio conversion</summary>
    OggEnc2 = 4,

    /// <summary>Audio encoding</summary>
    AudioEnc = 5
}
