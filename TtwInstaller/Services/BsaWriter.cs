using System.Collections.Concurrent;
using System.Runtime.InteropServices;
using System.Text.RegularExpressions;
using BsaLib;
using TtwInstaller.Models;

namespace TtwInstaller.Services;

/// <summary>
/// Handles BSA archive creation for TTW installation
/// Writes files to temp directories during processing, then packs BSAs at the end
/// </summary>
public class BsaWriter : IDisposable
{
    private readonly Dictionary<int, BsaArchive> _bsaArchives = new();
    private readonly string _destinationPath;
    private readonly string _tempBasePath;
    private bool _disposed;
    private readonly object _writeLock = new();

    public BsaWriter(List<Location> locations, string destinationPath)
    {
        _destinationPath = destinationPath;

        // Create a temp directory for staging BSA files next to the output directory
        // This avoids /tmp space limitations on systems with small tmpfs
        _tempBasePath = Path.Combine(destinationPath, ".ttw_bsa_staging_temp");
        Directory.CreateDirectory(_tempBasePath);

        // Identify BSA locations (Type 2 = WriteArchive with .bsa in name)
        for (int i = 0; i < locations.Count; i++)
        {
            var location = locations[i];

            if (location.Type == 2 && location.Name?.EndsWith(".bsa", StringComparison.OrdinalIgnoreCase) == true)
            {
                // Extract actual filename from Value (e.g., "%DESTINATION%\\New Fallout - Meshes.bsa" -> "New Fallout - Meshes.bsa")
                var bsaName = Path.GetFileName(location.Value?.Replace('\\', Path.DirectorySeparatorChar) ?? location.Name ?? "");

                // Use archive flags from location if available, otherwise use defaults
                var archiveFlags = location.ArchiveFlags.HasValue
                    ? (BsaInterop.ArchiveFlags)location.ArchiveFlags.Value
                    : GetArchiveFlags();

                // Use file flags (archive types) from location if available, otherwise derive from name
                var archiveTypes = location.FilesFlags.HasValue
                    ? (BsaInterop.ArchiveTypes)(ushort)location.FilesFlags.Value
                    : GetArchiveTypes(bsaName);

                // Create temp directory for this BSA
                var tempDir = Path.Combine(_tempBasePath, $"bsa_{i}");
                Directory.CreateDirectory(tempDir);

                _bsaArchives[i] = new BsaArchive
                {
                    Name = bsaName,
                    LocationIndex = i,
                    ArchiveFlags = archiveFlags,
                    ArchiveTypes = archiveTypes,
                    TempDirectory = tempDir,
                    FileCount = 0
                };

                Console.WriteLine($"  Registered BSA target: Location[{i}] = {bsaName} (Flags: 0x{(uint)archiveFlags:X}, Types: 0x{(ushort)archiveTypes:X})");
            }
        }

        Console.WriteLine($"Initialized {_bsaArchives.Count} BSA archives for packing");
        Console.WriteLine($"Temp staging directory: {_tempBasePath}\n");
    }

    /// <summary>
    /// Check if a location index is a BSA target
    /// </summary>
    public bool IsBsaLocation(int locationIndex)
    {
        return _bsaArchives.ContainsKey(locationIndex);
    }

    /// <summary>
    /// Add a file to a BSA collection (writes to temp directory instead of memory, thread-safe)
    /// </summary>
    public void AddFile(int locationIndex, string filePath, byte[] data)
    {
        if (!_bsaArchives.TryGetValue(locationIndex, out var bsa))
        {
            throw new InvalidOperationException($"Location {locationIndex} is not a BSA target");
        }

        // Normalize path (use forward slashes)
        var normalizedPath = NormalizePath(filePath);

        // Write file to temp directory structure
        var tempFilePath = Path.Combine(bsa.TempDirectory, normalizedPath.Replace('/', Path.DirectorySeparatorChar));
        var tempFileDir = Path.GetDirectoryName(tempFilePath);

        if (!string.IsNullOrEmpty(tempFileDir))
        {
            Directory.CreateDirectory(tempFileDir);
        }

        // Thread-safe file tracking and collision detection
        lock (_writeLock)
        {
            // Detect collision: check if temp file already exists
            if (File.Exists(tempFilePath))
            {
                // This is a collision! The temp path already exists.
                if (bsa.WrittenFiles.TryGetValue(tempFilePath, out var originalPath))
                {
                    bsa.Collisions.Add((originalPath, filePath, tempFilePath));
                }
            }
            else
            {
                // Track this file
                bsa.WrittenFiles[tempFilePath] = filePath;
            }

            File.WriteAllBytes(tempFilePath, data);
        }

        // Track file count (thread-safe)
        Interlocked.Increment(ref bsa.FileCount);
    }

    /// <summary>
    /// Write all collected BSA archives to disk
    /// </summary>
    public int WriteAllBsas()
    {
        Console.WriteLine($"\n=== Writing {_bsaArchives.Count} BSA Archives ===\n");

        int successCount = 0;
        int failCount = 0;

        foreach (var (locationIndex, bsa) in _bsaArchives.OrderBy(x => x.Key))
        {
            try
            {
                Console.Write($"  [{successCount + failCount + 1}/{_bsaArchives.Count}] {bsa.Name} ({bsa.FileCount} files) ... ");

                if (WriteBsa(bsa))
                {
                    Console.WriteLine("✅");
                    successCount++;
                }
                else
                {
                    Console.WriteLine("❌");
                    failCount++;
                }
            }
            catch (Exception ex)
            {
                Console.WriteLine($"❌ {ex.Message}");
                failCount++;
            }
        }

        Console.WriteLine($"\nBSA Creation: {successCount}/{_bsaArchives.Count} succeeded, {failCount} failed");

        // Report collisions
        ReportCollisions();

        return failCount;
    }

    /// <summary>
    /// Write a single BSA archive (reads files from temp directory)
    /// On Windows: Uses bsarch.exe
    /// On Linux: Uses libbsa
    /// </summary>
    private bool WriteBsa(BsaArchive bsa)
    {
        var outputPath = Path.Combine(_destinationPath, bsa.Name);

        // Windows: Use bsarch.exe
        if (RuntimeInformation.IsOSPlatform(OSPlatform.Windows))
        {
            // Extract flags as integers for bsarch command line
            int archiveFlags = (int)bsa.ArchiveFlags;
            int fileFlags = (int)bsa.ArchiveTypes;

            // Check if compression flag is set
            bool compress = (bsa.ArchiveFlags & BsaInterop.ArchiveFlags.Compressed) != 0;

            return BsarchWrapper.PackArchive(
                bsa.TempDirectory,
                outputPath,
                "fnv",
                archiveFlags,
                fileFlags,
                compress,
                multiThreaded: true);
        }

        // Linux: Use native libbsa
        IntPtr handle = IntPtr.Zero;
        try
        {
            // Create BSA
            handle = BsaInterop.bsa_create();
            if (handle == IntPtr.Zero)
            {
                var error = BsaInterop.bsa_get_last_error();
                throw new InvalidOperationException($"Failed to create BSA: {error}");
            }

            // Set archive flags and types
            BsaInterop.bsa_set_archive_flags(handle, bsa.ArchiveFlags);
            BsaInterop.bsa_set_archive_types(handle, bsa.ArchiveTypes);

            // Enumerate all files in temp directory
            var tempFiles = Directory.GetFiles(bsa.TempDirectory, "*", SearchOption.AllDirectories);

            foreach (var tempFilePath in tempFiles)
            {
                // Get relative path from temp directory
                var relativePath = Path.GetRelativePath(bsa.TempDirectory, tempFilePath);

                // Normalize to BSA format (forward slashes)
                var normalizedPath = relativePath.Replace(Path.DirectorySeparatorChar, '/');

                // Split into directory and filename
                var parts = SplitPath(normalizedPath);
                string dirPath = parts.Directory;
                string fileName = parts.FileName;

                // Read file data
                var fileData = File.ReadAllBytes(tempFilePath);

                // Add file to BSA
                int result = BsaInterop.bsa_add_file(
                    handle,
                    dirPath,
                    fileName,
                    fileData,
                    (nuint)fileData.Length);

                if (result != 0)
                {
                    var error = BsaInterop.bsa_get_last_error();
                    Console.WriteLine($"\n    Warning: Failed to add {normalizedPath}: {error}");
                }
            }

            // Write BSA to disk
            int writeResult = BsaInterop.bsa_write(
                handle,
                outputPath,
                BsaInterop.BSA_VERSION_FNV);

            if (writeResult != 0)
            {
                var error = BsaInterop.bsa_get_last_error();
                throw new InvalidOperationException($"Failed to write BSA: {error}");
            }

            return true;
        }
        finally
        {
            if (handle != IntPtr.Zero)
            {
                BsaInterop.bsa_free(handle);
            }
        }
    }

    /// <summary>
    /// Get standard archive flags for FNV BSAs
    /// </summary>
    private static BsaInterop.ArchiveFlags GetArchiveFlags()
    {
        // Standard flags for Fallout New Vegas BSAs
        return BsaInterop.ArchiveFlags.DirectoryStrings |
               BsaInterop.ArchiveFlags.FileStrings |
               BsaInterop.ArchiveFlags.Compressed |
               BsaInterop.ArchiveFlags.RetainDirectoryNames |
               BsaInterop.ArchiveFlags.RetainFileNames |
               BsaInterop.ArchiveFlags.RetainFileNameOffsets;
    }

    /// <summary>
    /// Derive archive types from BSA name
    /// </summary>
    private static BsaInterop.ArchiveTypes GetArchiveTypes(string bsaName)
    {
        var nameLower = bsaName.ToLowerInvariant();

        // Check for specific content types based on BSA name
        if (nameLower.Contains("meshes"))
            return BsaInterop.ArchiveTypes.Meshes;

        if (nameLower.Contains("textures"))
            return BsaInterop.ArchiveTypes.Textures;

        if (nameLower.Contains("menuvoices"))
            return BsaInterop.ArchiveTypes.Menus | BsaInterop.ArchiveTypes.Voices;

        if (nameLower.Contains("voices"))
            return BsaInterop.ArchiveTypes.Voices;

        if (nameLower.Contains("sound"))
            return BsaInterop.ArchiveTypes.Sounds;

        if (nameLower.Contains("main") || nameLower.Contains("misc"))
            return BsaInterop.ArchiveTypes.Misc;

        // Default to Misc for unknown types
        return BsaInterop.ArchiveTypes.Misc;
    }

    /// <summary>
    /// Normalize file path for BSA (lowercase, forward slashes)
    /// </summary>
    private static string NormalizePath(string path)
    {
        // Convert backslashes to forward slashes
        var normalized = path.Replace('\\', '/');

        // Remove leading ./ if present
        if (normalized.StartsWith("./"))
        {
            normalized = normalized.Substring(2);
        }

        // Remove leading slashes
        normalized = normalized.TrimStart('/');

        // Convert to lowercase to match BSA case-insensitive behavior
        // This ensures collision detection works correctly on case-sensitive filesystems (Linux)
        normalized = normalized.ToLowerInvariant();

        return normalized;
    }

    /// <summary>
    /// Split a path into directory and filename components
    /// </summary>
    private static (string Directory, string FileName) SplitPath(string path)
    {
        var normalized = NormalizePath(path);
        var lastSlash = normalized.LastIndexOf('/');

        if (lastSlash == -1)
        {
            // No directory, just filename
            return (string.Empty, normalized);
        }

        var directory = normalized.Substring(0, lastSlash);
        var fileName = normalized.Substring(lastSlash + 1);

        return (directory, fileName);
    }

    /// <summary>
    /// Report file path collisions that occurred during BSA creation
    /// </summary>
    private void ReportCollisions()
    {
        int totalCollisions = 0;
        var collisionReport = new List<string>();

        foreach (var (locationIndex, bsa) in _bsaArchives.OrderBy(x => x.Key))
        {
            if (bsa.Collisions.Count > 0)
            {
                totalCollisions += bsa.Collisions.Count;
                collisionReport.Add($"\n=== {bsa.Name} - {bsa.Collisions.Count} Collisions ===");

                foreach (var (original, duplicate, tempPath) in bsa.Collisions.OrderBy(c => c.TempPath))
                {
                    collisionReport.Add($"  Collision at: {Path.GetRelativePath(bsa.TempDirectory, tempPath)}");
                    collisionReport.Add($"    Original:  {original}");
                    collisionReport.Add($"    Duplicate: {duplicate}");
                }
            }
        }

        if (totalCollisions > 0)
        {
            Console.WriteLine($"\n⚠️  WARNING: {totalCollisions} file path collisions detected!");
            Console.WriteLine("   Files with different cases overwriting each other due to case-insensitive filesystem.");

            // Save detailed report
            var reportPath = Path.Combine(_destinationPath, "bsa_collisions_report.txt");
            File.WriteAllLines(reportPath, collisionReport);
            Console.WriteLine($"   Detailed collision report saved to: {reportPath}");

            // Print summary
            foreach (var line in collisionReport.Take(50))
            {
                Console.WriteLine(line);
            }
            if (collisionReport.Count > 50)
            {
                Console.WriteLine($"   ... and {collisionReport.Count - 50} more lines (see report file)");
            }
        }
    }

    public void Dispose()
    {
        if (!_disposed)
        {
            // Clean up temp staging directory
            try
            {
                if (Directory.Exists(_tempBasePath))
                {
                    Directory.Delete(_tempBasePath, recursive: true);
                    Console.WriteLine($"Cleaned up temp staging directory: {_tempBasePath}");
                }
            }
            catch (Exception ex)
            {
                Console.WriteLine($"Warning: Failed to clean up temp directory: {ex.Message}");
            }

            _disposed = true;
        }
        GC.SuppressFinalize(this);
    }

    /// <summary>
    /// Container for BSA archive data
    /// </summary>
    private class BsaArchive
    {
        public required string Name { get; init; }
        public required int LocationIndex { get; init; }
        public required BsaInterop.ArchiveFlags ArchiveFlags { get; init; }
        public required BsaInterop.ArchiveTypes ArchiveTypes { get; init; }
        public required string TempDirectory { get; init; }
        public int FileCount;
        public ConcurrentDictionary<string, string> WrittenFiles = new(); // temp path -> original path
        public ConcurrentBag<(string Original, string Duplicate, string TempPath)> Collisions = new();
    }
}
