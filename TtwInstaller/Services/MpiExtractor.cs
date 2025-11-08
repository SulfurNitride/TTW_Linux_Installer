using Mutagen.Bethesda;
using Mutagen.Bethesda.Archives;

namespace TtwInstaller.Services;

/// <summary>
/// Extracts .mpi files (BSA archives) to a temporary directory
/// </summary>
public class MpiExtractor
{
    /// <summary>
    /// Check if the path is an .mpi file that needs extraction
    /// </summary>
    public static bool IsMpiFile(string path)
    {
        return File.Exists(path) && path.EndsWith(".mpi", StringComparison.OrdinalIgnoreCase);
    }

    /// <summary>
    /// Extract .mpi file to a temporary directory
    /// Returns the path to the extracted directory
    /// </summary>
    public static string ExtractMpiToTemp(string mpiPath)
    {
        if (!File.Exists(mpiPath))
        {
            throw new FileNotFoundException($"MPI file not found: {mpiPath}");
        }

        Console.WriteLine($"\nExtracting MPI package: {Path.GetFileName(mpiPath)}");
        Console.WriteLine($"This may take a few minutes...\n");

        // Create temp directory for extraction
        var tempDir = Path.Combine(Path.GetTempPath(), $"ttw_mpi_{Guid.NewGuid():N}");
        Directory.CreateDirectory(tempDir);

        // Register for cleanup on app exit
        TempDirectoryTracker.Register(tempDir);

        try
        {
            // Open the BSA/MPI archive using Mutagen
            // Use SkyrimLE for V104 BSA format (same as FO3/FNV)
            var archive = Archive.CreateReader(GameRelease.SkyrimLE, mpiPath);

            Console.WriteLine($"Archive opened: {archive.Files.Count()} files found");

            int extracted = 0;
            int failed = 0;

            foreach (var file in archive.Files)
            {
                try
                {
                    var outputPath = Path.Combine(tempDir, file.Path);
                    var directory = Path.GetDirectoryName(outputPath);

                    if (!string.IsNullOrEmpty(directory))
                    {
                        Directory.CreateDirectory(directory);
                    }

                    using var stream = file.AsStream();
                    using var outputFile = File.Create(outputPath);
                    stream.CopyTo(outputFile);

                    extracted++;

                    if (extracted % 100 == 0)
                    {
                        Console.WriteLine($"  Extracted {extracted}/{archive.Files.Count()} files...");
                    }
                }
                catch (Exception ex)
                {
                    Console.WriteLine($"  Warning: Failed to extract {file.Path}: {ex.Message}");
                    failed++;
                }
            }

            Console.WriteLine($"\n✅ MPI extraction complete: {extracted} files extracted");
            if (failed > 0)
            {
                Console.WriteLine($"⚠️  {failed} files failed to extract\n");
            }

            return tempDir;
        }
        catch (Exception ex)
        {
            // Clean up on failure
            try
            {
                if (Directory.Exists(tempDir))
                {
                    Directory.Delete(tempDir, true);
                }
            }
            catch { }

            throw new InvalidOperationException($"Failed to extract MPI file: {ex.Message}", ex);
        }
    }

    /// <summary>
    /// Clean up a temporary extraction directory
    /// </summary>
    public static void CleanupTempDirectory(string tempDir)
    {
        if (string.IsNullOrEmpty(tempDir) || !tempDir.Contains("ttw_mpi_"))
        {
            return; // Safety check - only delete our temp directories
        }

        try
        {
            if (Directory.Exists(tempDir))
            {
                Console.WriteLine($"\nCleaning up temporary files...");
                Directory.Delete(tempDir, true);
                // Unregister from tracker since we successfully cleaned it up
                TempDirectoryTracker.Unregister(tempDir);
            }
        }
        catch (Exception ex)
        {
            Console.WriteLine($"Warning: Failed to clean up temp directory: {ex.Message}");
        }
    }
}
