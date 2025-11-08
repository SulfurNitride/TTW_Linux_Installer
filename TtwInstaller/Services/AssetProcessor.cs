using TtwInstaller.Models;

namespace TtwInstaller.Services;

/// <summary>
/// Processes installation assets
/// </summary>
public class AssetProcessor
{
    private readonly LocationResolver _locationResolver;
    private readonly BsaReader _bsaReader;
    private readonly InstallConfig _config;
    private readonly BsaWriter? _bsaWriter;
    private readonly InstallationLogger? _logger;

    public AssetProcessor(LocationResolver locationResolver, BsaReader bsaReader, InstallConfig config, BsaWriter? bsaWriter = null, InstallationLogger? logger = null)
    {
        _locationResolver = locationResolver;
        _bsaReader = bsaReader;
        _config = config;
        _bsaWriter = bsaWriter;
        _logger = logger;
    }

    /// <summary>
    /// Process a Copy operation (OpType 0)
    /// </summary>
    public bool ProcessCopy(Asset asset)
    {
        var sourceLocation = _locationResolver.GetLocation(asset.SourceLoc);
        var targetPath = Path.Combine(
            _locationResolver.GetDirectoryPath(asset.TargetLoc),
            asset.GetEffectiveTargetPath()
        );

        byte[]? fileData = null;

        // Source can be a BSA or directory
        if (_locationResolver.IsBsaLocation(asset.SourceLoc))
        {
            // Extract from BSA
            var bsaPath = _locationResolver.GetBsaPath(asset.SourceLoc);
            fileData = _bsaReader.ExtractFile(bsaPath, asset.SourcePath);

            if (fileData == null)
            {
                _logger?.LogMissingFile(asset.SourcePath, $"Not found in BSA: {bsaPath}");
                Console.WriteLine($"  ⚠️  File not found in BSA: {asset.SourcePath}");
                return false;
            }
        }
        else
        {
            // Copy from directory
            // Normalize path separators for cross-platform compatibility
            var normalizedSourcePath = asset.SourcePath.Replace('\\', Path.DirectorySeparatorChar);
            var sourcePath = Path.Combine(
                _locationResolver.ResolvePath(asset.SourceLoc),
                normalizedSourcePath
            );

            // Try case-sensitive first (Windows)
            if (!File.Exists(sourcePath))
            {
                // On Linux, try case-insensitive lookup
                var foundPath = FindFileCaseInsensitive(sourcePath);
                if (foundPath == null)
                {
                    Console.WriteLine($"  ⚠️  Source file not found in MPI (tried: {sourcePath})");
                    return false;
                }
                sourcePath = foundPath;
            }

            fileData = File.ReadAllBytes(sourcePath);
        }

        // Write to target (BSA or disk)
        WriteFileData(asset, fileData);
        return true;
    }

    /// <summary>
    /// Process a New operation (OpType 1)
    /// </summary>
    public bool ProcessNew(Asset asset)
    {
        // New files come from the MPI package
        // Normalize path separators for cross-platform compatibility
        var normalizedPath = asset.SourcePath.Replace('\\', Path.DirectorySeparatorChar);
        var sourcePath = Path.Combine(_config.MpiPackagePath, normalizedPath);

        // Try case-sensitive first (Windows)
        if (!File.Exists(sourcePath))
        {
            // On Linux, try case-insensitive lookup
            var foundPath = FindFileCaseInsensitive(sourcePath);
            if (foundPath == null)
            {
                Console.WriteLine($"  ⚠️  Source file not found in MPI (tried: {sourcePath})");
                return false;
            }
            sourcePath = foundPath;
        }

        // Read file data and write to target (BSA or disk)
        var fileData = File.ReadAllBytes(sourcePath);
        WriteFileData(asset, fileData);
        return true;
    }

    /// <summary>
    /// Process an asset based on its operation type
    /// </summary>
    public bool ProcessAsset(Asset asset)
    {
        return asset.OpType switch
        {
            0 => ProcessCopy(asset),
            1 => ProcessNew(asset),
            2 => ProcessPatch(asset),
            4 => ProcessOggEnc2(asset),
            5 => ProcessAudioEnc(asset),
            _ => throw new NotImplementedException($"Operation type {asset.OpType} not implemented")
        };
    }

    /// <summary>
    /// Process a Patch operation (OpType 2) - Apply .xd3 binary patch
    /// </summary>
    private bool ProcessPatch(Asset asset)
    {
        try
        {
            // Get the patch file from MPI (e.g., "fallout3.esm.xd3")
            // The patch file name is based on the TARGET path, not source!
            var patchFileName = asset.GetEffectiveTargetPath() + ".xd3";
            var normalizedPatchPath = patchFileName.Replace('\\', Path.DirectorySeparatorChar);
            var patchFile = Path.Combine(_config.MpiPackagePath, normalizedPatchPath);

            // Try case-insensitive lookup if not found
            if (!File.Exists(patchFile))
            {
                var foundPath = FindFileCaseInsensitive(patchFile);
                if (foundPath == null)
                {
                    Console.WriteLine($"  ⚠️  Patch file not found: {patchFile}");
                    return false;
                }
                patchFile = foundPath;
            }

            // Get the source file to patch (from FO3/FNV game directory)
            // The source is typically in the game's Data folder
            byte[]? sourceData = null;

            // Check if source is from BSA or directory
            if (_locationResolver.IsBsaLocation(asset.SourceLoc))
            {
                // Extract from BSA
                var bsaPath = _locationResolver.GetBsaPath(asset.SourceLoc);
                // asset.SourcePath contains the original filename without .xd3
                // This is the file we need to extract from the BSA
                sourceData = _bsaReader.ExtractFile(bsaPath, asset.SourcePath);

                if (sourceData == null)
                {
                    // Try from game directory instead
                    var gameDir = _locationResolver.GetDirectoryPath(asset.SourceLoc);
                    var normalizedPath = asset.SourcePath.Replace('\\', Path.DirectorySeparatorChar);
                    var sourceFile = Path.Combine(gameDir, normalizedPath);

                    if (File.Exists(sourceFile))
                    {
                        sourceData = File.ReadAllBytes(sourceFile);
                    }
                }
            }
            else
            {
                // Source from directory
                var gameDir = _locationResolver.GetDirectoryPath(asset.SourceLoc);
                // asset.SourcePath contains the original filename without .xd3
                // Use it directly - it already has the correct extension (.esm, .nif, etc.)
                var normalizedSourcePath = asset.SourcePath.Replace('\\', Path.DirectorySeparatorChar);
                var sourceFile = Path.Combine(gameDir, normalizedSourcePath);

                if (File.Exists(sourceFile))
                {
                    sourceData = File.ReadAllBytes(sourceFile);
                }
            }

            if (sourceData == null)
            {
                Console.WriteLine($"  ⚠️  Source file for patching not found: {asset.SourcePath}");
                Console.WriteLine($"       This file is required to apply the patch.");
                return false;
            }

            // Log source file checksum for debugging
            string sourceChecksum = TtwInstaller.Utils.ChecksumHelper.CalculateSHA1(sourceData);
            Console.WriteLine($"       Source file SHA1: {sourceChecksum.Substring(0, 16)}...");

            // Check if patch file is LZ4-compressed (magic bytes: 0x04 0x22 0x4D 0x18)
            var patchData = File.ReadAllBytes(patchFile);
            bool isLz4Compressed = patchData.Length >= 4 &&
                                   patchData[0] == 0x04 &&
                                   patchData[1] == 0x22 &&
                                   patchData[2] == 0x4D &&
                                   patchData[3] == 0x18;

            string actualPatchFile = patchFile;

            if (isLz4Compressed)
            {
                Console.WriteLine($"       Patch is LZ4-compressed, decompressing...");

                // Decompress LZ4 patch to temp file using lz4 command-line tool
                var tempDecompressedPatch = Path.GetTempFileName();
                try
                {
                    // Use lz4 command to decompress (with -f to force overwrite)
                    var lz4Process = new System.Diagnostics.Process
                    {
                        StartInfo = new System.Diagnostics.ProcessStartInfo
                        {
                            FileName = "lz4",
                            Arguments = $"-d -f \"{patchFile}\" \"{tempDecompressedPatch}\"",
                            RedirectStandardOutput = true,
                            RedirectStandardError = true,
                            UseShellExecute = false,
                            CreateNoWindow = true
                        }
                    };

                    lz4Process.Start();
                    lz4Process.WaitForExit();

                    if (lz4Process.ExitCode != 0)
                    {
                        var error = lz4Process.StandardError.ReadToEnd();
                        Console.WriteLine($"  ⚠️  LZ4 decompression failed: {error}");
                        if (File.Exists(tempDecompressedPatch))
                            File.Delete(tempDecompressedPatch);
                        return false;
                    }

                    actualPatchFile = tempDecompressedPatch;
                    Console.WriteLine($"       LZ4 decompression successful");
                }
                catch (Exception ex)
                {
                    Console.WriteLine($"  ⚠️  LZ4 decompression failed: {ex.Message}");
                    if (File.Exists(tempDecompressedPatch))
                        File.Delete(tempDecompressedPatch);
                    return false;
                }
            }

            // Create temp files for xdelta3
            var tempSourceFile = Path.GetTempFileName();
            var tempOutputFile = Path.GetTempFileName();

            try
            {
                // Write source data to temp file
                File.WriteAllBytes(tempSourceFile, sourceData);

                // Apply patch using xdelta3
                var process = new System.Diagnostics.Process
                {
                    StartInfo = new System.Diagnostics.ProcessStartInfo
                    {
                        FileName = BundledBinaryManager.GetXdelta3Path(),
                        Arguments = $"-d -f -s \"{tempSourceFile}\" \"{actualPatchFile}\" \"{tempOutputFile}\"",
                        RedirectStandardOutput = true,
                        RedirectStandardError = true,
                        UseShellExecute = false,
                        CreateNoWindow = true
                    }
                };

                process.Start();
                process.WaitForExit();

                if (process.ExitCode != 0)
                {
                    var error = process.StandardError.ReadToEnd();
                    var output = process.StandardOutput.ReadToEnd();

                    Console.WriteLine($"  ⚠️  xdelta3 patch failed:");
                    Console.WriteLine($"       Error: {error.Trim()}");

                    if (!string.IsNullOrEmpty(output))
                    {
                        Console.WriteLine($"       Output: {output.Trim()}");
                    }

                    // Provide helpful context
                    if (error.Contains("source file too short") || error.Contains("checksum mismatch"))
                    {
                        Console.WriteLine($"       This usually means the source file doesn't match the expected version.");
                        Console.WriteLine($"       The patch was created for a different version of '{asset.SourcePath}'.");
                        Console.WriteLine($"       Try reinstalling your game to get the correct file versions.");
                    }

                    return false;
                }

                // Read patched data and write to target (BSA or disk)
                var patchedData = File.ReadAllBytes(tempOutputFile);
                WriteFileData(asset, patchedData);
                return true;
            }
            finally
            {
                // Clean up temp files
                if (File.Exists(tempSourceFile)) File.Delete(tempSourceFile);
                if (File.Exists(tempOutputFile)) File.Delete(tempOutputFile);

                // Clean up decompressed LZ4 patch file if it was created
                if (isLz4Compressed && actualPatchFile != patchFile && File.Exists(actualPatchFile))
                    File.Delete(actualPatchFile);
            }
        }
        catch (Exception ex)
        {
            Console.WriteLine($"  ⚠️  Patch error: {ex.Message}");
            return false;
        }
    }

    /// <summary>
    /// Process OggEnc2 operation (OpType 4) - Resample OGG audio using FFmpeg
    /// </summary>
    private bool ProcessOggEnc2(Asset asset)
    {
        try
        {
            // Parse params to get frequency (format: "-f:24000 -q:5")
            var frequency = ParseAudioParams(asset.Params)
                .GetValueOrDefault("f", "24000"); // Default to 24kHz

            // Get source audio data
            byte[]? sourceData = null;

            if (_locationResolver.IsBsaLocation(asset.SourceLoc))
            {
                var bsaPath = _locationResolver.GetBsaPath(asset.SourceLoc);
                sourceData = _bsaReader.ExtractFile(bsaPath, asset.SourcePath);
            }
            else
            {
                var sourceDir = _locationResolver.GetDirectoryPath(asset.SourceLoc);
                var normalizedPath = asset.SourcePath.Replace('\\', Path.DirectorySeparatorChar);
                var sourceFile = Path.Combine(sourceDir, normalizedPath);

                if (File.Exists(sourceFile))
                {
                    sourceData = File.ReadAllBytes(sourceFile);
                }
            }

            if (sourceData == null)
            {
                Console.WriteLine($"  ⚠️  Source file not found: {asset.SourcePath}");
                return false;
            }

            // Create temp files for FFmpeg (proper cleanup - don't use GetTempFileName)
            var tempInput = Path.Combine(Path.GetTempPath(), $"ttw_ogg_{Guid.NewGuid():N}.ogg");
            var tempOutput = Path.Combine(Path.GetTempPath(), $"ttw_ogg_{Guid.NewGuid():N}.ogg");

            try
            {
                // Write source to temp file
                File.WriteAllBytes(tempInput, sourceData);

                // Run FFmpeg to resample OGG
                // -nostdin: prevent hanging on stdin
                // -hide_banner -loglevel error: reduce output
                // -i input: input file
                // -ar frequency: set audio sampling rate
                // -c:a libvorbis: use vorbis codec
                // -y: overwrite output
                var process = new System.Diagnostics.Process
                {
                    StartInfo = new System.Diagnostics.ProcessStartInfo
                    {
                        FileName = "ffmpeg",
                        Arguments = $"-nostdin -hide_banner -loglevel error -i \"{tempInput}\" -ar {frequency} -c:a libvorbis -y \"{tempOutput}\"",
                        RedirectStandardOutput = true,
                        RedirectStandardError = true,
                        UseShellExecute = false,
                        CreateNoWindow = true
                    }
                };

                process.Start();

                // Read output streams asynchronously to prevent deadlock
                var outputTask = process.StandardOutput.ReadToEndAsync();
                var errorTask = process.StandardError.ReadToEndAsync();

                // Wait with timeout (30 seconds per file)
                if (!process.WaitForExit(30000))
                {
                    process.Kill();
                    Console.WriteLine($"  ⚠️  FFmpeg timeout (30s) - killed process");
                    return false;
                }

                // Ensure async reads complete
                var stderr = errorTask.Result;

                if (process.ExitCode != 0)
                {
                    Console.WriteLine($"  ⚠️  FFmpeg failed (exit {process.ExitCode}): {stderr}");
                    return false;
                }

                // Read resampled audio and write to target (BSA or disk)
                var resampledData = File.ReadAllBytes(tempOutput);
                WriteFileData(asset, resampledData);
                return true;
            }
            finally
            {
                // Clean up temp files
                try { if (File.Exists(tempInput)) File.Delete(tempInput); } catch { }
                try { if (File.Exists(tempOutput)) File.Delete(tempOutput); } catch { }
            }
        }
        catch (Exception ex)
        {
            Console.WriteLine($"  ⚠️  OggEnc2 error: {ex.Message}");
            return false;
        }
    }

    /// <summary>
    /// Process AudioEnc operation (OpType 5) - Convert audio formats using FFmpeg
    /// </summary>
    private bool ProcessAudioEnc(Asset asset)
    {
        try
        {
            // Parse params (format: "-fmt:wav -f:44100 -c:2 -b:192")
            var audioParams = ParseAudioParams(asset.Params);

            // Get source audio data
            byte[]? sourceData = null;

            if (_locationResolver.IsBsaLocation(asset.SourceLoc))
            {
                var bsaPath = _locationResolver.GetBsaPath(asset.SourceLoc);
                sourceData = _bsaReader.ExtractFile(bsaPath, asset.SourcePath);
            }
            else
            {
                var sourceDir = _locationResolver.GetDirectoryPath(asset.SourceLoc);
                var normalizedPath = asset.SourcePath.Replace('\\', Path.DirectorySeparatorChar);
                var sourceFile = Path.Combine(sourceDir, normalizedPath);

                if (File.Exists(sourceFile))
                {
                    sourceData = File.ReadAllBytes(sourceFile);
                }
            }

            if (sourceData == null)
            {
                Console.WriteLine($"  ⚠️  Source file not found: {asset.SourcePath}");
                return false;
            }

            // Determine target format from extension
            var targetPath = Path.Combine(
                _locationResolver.GetDirectoryPath(asset.TargetLoc),
                asset.GetEffectiveTargetPath()
            );
            var targetExtension = Path.GetExtension(targetPath).TrimStart('.').ToLower();

            // Determine source extension
            var sourceExtension = Path.GetExtension(asset.SourcePath).TrimStart('.').ToLower();
            if (string.IsNullOrEmpty(sourceExtension))
                sourceExtension = "wav"; // Default if no extension

            // Create temp files with proper extensions (proper cleanup - don't use GetTempFileName)
            var tempInput = Path.Combine(Path.GetTempPath(), $"ttw_audio_{Guid.NewGuid():N}.{sourceExtension}");
            var tempOutput = Path.Combine(Path.GetTempPath(), $"ttw_audio_{Guid.NewGuid():N}.{targetExtension}");

            try
            {
                // Write source to temp file
                File.WriteAllBytes(tempInput, sourceData);

                // Build FFmpeg arguments based on target format
                var ffmpegArgs = new List<string>
                {
                    "-nostdin",  // Prevent ffmpeg from reading from stdin (fixes hanging)
                    "-hide_banner",  // Reduce output verbosity
                    "-loglevel error",  // Only show errors
                    $"-i \"{tempInput}\""
                };

                // Add frequency if specified
                if (audioParams.TryGetValue("f", out var frequency))
                {
                    ffmpegArgs.Add($"-ar {frequency}");
                }

                // Add channels if specified
                if (audioParams.TryGetValue("c", out var channels))
                {
                    ffmpegArgs.Add($"-ac {channels}");
                }

                // Add format-specific options
                if (targetExtension == "mp3")
                {
                    ffmpegArgs.Add("-c:a libmp3lame");

                    // Add bitrate if specified
                    if (audioParams.TryGetValue("b", out var bitrate))
                    {
                        ffmpegArgs.Add($"-b:a {bitrate}k");
                    }
                }
                else if (targetExtension == "wav")
                {
                    ffmpegArgs.Add("-c:a pcm_s16le");
                }
                else if (targetExtension == "ogg")
                {
                    ffmpegArgs.Add("-c:a libvorbis");
                }

                ffmpegArgs.Add("-y");
                ffmpegArgs.Add($"\"{tempOutput}\"");

                var process = new System.Diagnostics.Process
                {
                    StartInfo = new System.Diagnostics.ProcessStartInfo
                    {
                        FileName = "ffmpeg",
                        Arguments = string.Join(" ", ffmpegArgs),
                        RedirectStandardOutput = true,
                        RedirectStandardError = true,
                        UseShellExecute = false,
                        CreateNoWindow = true
                    }
                };

                process.Start();

                // Read output streams asynchronously to prevent deadlock
                var outputTask = process.StandardOutput.ReadToEndAsync();
                var errorTask = process.StandardError.ReadToEndAsync();

                // Wait with timeout (30 seconds per file should be plenty)
                if (!process.WaitForExit(30000))
                {
                    process.Kill();
                    Console.WriteLine($"  ⚠️  FFmpeg timeout (30s) - killed process");
                    return false;
                }

                // Ensure async reads complete
                var stderr = errorTask.Result;

                if (process.ExitCode != 0)
                {
                    Console.WriteLine($"  ⚠️  FFmpeg failed (exit {process.ExitCode}): {stderr}");
                    return false;
                }

                // Read converted audio and write to target (BSA or disk)
                var convertedData = File.ReadAllBytes(tempOutput);
                WriteFileData(asset, convertedData);
                return true;
            }
            finally
            {
                // Clean up temp files (catch exceptions to prevent cleanup failures from killing the process)
                try { if (File.Exists(tempInput)) File.Delete(tempInput); } catch { }
                try { if (File.Exists(tempOutput)) File.Delete(tempOutput); } catch { }
            }
        }
        catch (Exception ex)
        {
            Console.WriteLine($"  ⚠️  AudioEnc error: {ex.Message}");
            return false;
        }
    }

    /// <summary>
    /// Parse audio params string (format: "-key:value -key:value")
    /// </summary>
    private Dictionary<string, string> ParseAudioParams(string paramsString)
    {
        var result = new Dictionary<string, string>();

        if (string.IsNullOrWhiteSpace(paramsString))
            return result;

        var parts = paramsString.Split(new[] { ' ' }, StringSplitOptions.RemoveEmptyEntries);

        foreach (var part in parts)
        {
            if (part.StartsWith("-") && part.Contains(":"))
            {
                var colonIndex = part.IndexOf(':');
                var key = part.Substring(1, colonIndex - 1);
                var value = part.Substring(colonIndex + 1);
                result[key] = value;
            }
        }

        return result;
    }

    /// <summary>
    /// Write file data to either BSA or disk depending on target location
    /// </summary>
    private void WriteFileData(Asset asset, byte[] fileData)
    {
        // Normalize target path (strip .\ or ./ prefix)
        var filePath = NormalizeRelativePath(asset.GetEffectiveTargetPath());

        // Check if target location is a BSA
        if (_bsaWriter != null && _bsaWriter.IsBsaLocation(asset.TargetLoc))
        {
            // Add to BSA collection
            _bsaWriter.AddFile(asset.TargetLoc, filePath, fileData);
        }
        else
        {
            // Write to disk
            var targetPath = Path.Combine(
                _locationResolver.GetDirectoryPath(asset.TargetLoc),
                filePath
            );

            var normalizedTargetPath = targetPath.Replace('\\', Path.DirectorySeparatorChar);
            var targetDir = Path.GetDirectoryName(normalizedTargetPath);

            if (!string.IsNullOrEmpty(targetDir))
            {
                Directory.CreateDirectory(targetDir);
            }

            File.WriteAllBytes(normalizedTargetPath, fileData);
        }
    }

    /// <summary>
    /// Find a file using case-insensitive matching (for Linux compatibility)
    /// </summary>
    private string? FindFileCaseInsensitive(string path)
    {
        try
        {
            // If file exists with exact case, return it
            if (File.Exists(path))
                return path;

            var directory = Path.GetDirectoryName(path);
            var fileName = Path.GetFileName(path);

            if (string.IsNullOrEmpty(directory) || string.IsNullOrEmpty(fileName))
                return null;

            // Check if directory exists (case-sensitive first)
            if (!Directory.Exists(directory))
            {
                // Try to find directory case-insensitively
                directory = FindDirectoryCaseInsensitive(directory);
                if (directory == null)
                    return null;
            }

            // Find file in directory (case-insensitive)
            var files = Directory.GetFiles(directory);
            var match = files.FirstOrDefault(f =>
                string.Equals(Path.GetFileName(f), fileName, StringComparison.OrdinalIgnoreCase));

            return match;
        }
        catch
        {
            return null;
        }
    }

    /// <summary>
    /// Find a directory using case-insensitive matching (recursive)
    /// </summary>
    private string? FindDirectoryCaseInsensitive(string path)
    {
        try
        {
            if (Directory.Exists(path))
                return path;

            var parent = Path.GetDirectoryName(path);
            var dirName = Path.GetFileName(path);

            if (string.IsNullOrEmpty(parent) || string.IsNullOrEmpty(dirName))
                return null;

            // Recursively find parent
            if (!Directory.Exists(parent))
            {
                parent = FindDirectoryCaseInsensitive(parent);
                if (parent == null)
                    return null;
            }

            // Find subdirectory in parent (case-insensitive)
            var dirs = Directory.GetDirectories(parent);
            var match = dirs.FirstOrDefault(d =>
                string.Equals(Path.GetFileName(d), dirName, StringComparison.OrdinalIgnoreCase));

            return match;
        }
        catch
        {
            return null;
        }
    }

    /// <summary>
    /// Normalize a relative path by stripping .\ or ./ prefix
    /// </summary>
    private static string NormalizeRelativePath(string path)
    {
        if (string.IsNullOrEmpty(path))
            return path;

        // Strip leading .\ or ./
        if (path.StartsWith(".\\") || path.StartsWith("./"))
        {
            path = path.Substring(2);
        }

        // Strip leading \ or /
        while (path.StartsWith("\\") || path.StartsWith("/"))
        {
            path = path.Substring(1);
        }

        return path;
    }
}
