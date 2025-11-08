using System.Text.Json.Serialization;

namespace TtwInstaller.Models;

/// <summary>
/// Root manifest structure for TTW installation
/// </summary>
public class TtwManifest
{
    [JsonPropertyName("Package")]
    public PackageInfo? Package { get; set; }

    [JsonPropertyName("Variables")]
    public List<List<Variable>>? Variables { get; set; }

    [JsonPropertyName("Locations")]
    public List<List<Location>>? Locations { get; set; }

    [JsonPropertyName("Tags")]
    public List<Tag>? Tags { get; set; }

    [JsonPropertyName("Assets")]
    public List<object>? Assets { get; set; }

    [JsonPropertyName("Checks")]
    public List<Check>? Checks { get; set; }

    [JsonPropertyName("FileAttrs")]
    public List<FileAttr>? FileAttrs { get; set; }

    [JsonPropertyName("PostCommands")]
    public List<PostCommand>? PostCommands { get; set; }
}

public class PackageInfo
{
    [JsonPropertyName("Title")]
    public string? Title { get; set; }

    [JsonPropertyName("Version")]
    public string? Version { get; set; }

    [JsonPropertyName("Author")]
    public string? Author { get; set; }

    [JsonPropertyName("HomePage")]
    public string? HomePage { get; set; }

    [JsonPropertyName("Description")]
    public string? Description { get; set; }
}

public class Variable
{
    [JsonPropertyName("Name")]
    public string? Name { get; set; }

    [JsonPropertyName("Type")]
    public int Type { get; set; }

    [JsonPropertyName("Value")]
    public string? Value { get; set; }

    [JsonPropertyName("ExcludeDelimiter")]
    public bool ExcludeDelimiter { get; set; }
}

public class Location
{
    [JsonPropertyName("Name")]
    public string? Name { get; set; }

    [JsonPropertyName("Type")]
    public int Type { get; set; }

    [JsonPropertyName("Value")]
    public string? Value { get; set; }

    [JsonPropertyName("CreateFolder")]
    public bool? CreateFolder { get; set; }

    // BSA-specific properties (Type = 2)
    [JsonPropertyName("ArchiveType")]
    public ushort? ArchiveType { get; set; }

    [JsonPropertyName("ArchiveFlags")]
    public uint? ArchiveFlags { get; set; }

    [JsonPropertyName("FilesFlags")]
    public uint? FilesFlags { get; set; }

    [JsonPropertyName("ArchiveCompressed")]
    public bool? ArchiveCompressed { get; set; }
}

public class Tag
{
    [JsonPropertyName("Name")]
    public string? Name { get; set; }

    [JsonPropertyName("ID")]
    public int ID { get; set; }

    [JsonPropertyName("TextColor")]
    public string? TextColor { get; set; }

    [JsonPropertyName("BackColor")]
    public string? BackColor { get; set; }
}

public class Check
{
    [JsonPropertyName("Type")]
    public int Type { get; set; }

    [JsonPropertyName("Inverted")]
    public bool Inverted { get; set; }

    [JsonPropertyName("Loc")]
    public int Loc { get; set; }

    [JsonPropertyName("File")]
    public string? File { get; set; }

    [JsonPropertyName("Checksums")]
    public string? Checksums { get; set; }

    [JsonPropertyName("FreeSize")]
    public long? FreeSize { get; set; }

    [JsonPropertyName("CustomMessage")]
    public string? CustomMessage { get; set; }
}

public class FileAttr
{
    [JsonPropertyName("Value")]
    public string? Value { get; set; }

    [JsonPropertyName("LastModified")]
    public DateTime? LastModified { get; set; }
}

public class PostCommand
{
    [JsonPropertyName("Value")]
    public string? Value { get; set; }

    [JsonPropertyName("Wait")]
    public bool Wait { get; set; }

    [JsonPropertyName("Hidden")]
    public bool Hidden { get; set; }
}
